//! `stitch`: the single binary (design §2.1). `stitch node` runs the node;
//! every other command is a client of the node of its data directory,
//! starting it in the background when none runs, or reads the store and
//! index directly.

use std::error::Error;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use stitch_net::{
  BlobPolicy, NodeState, Peer, Peers, Policy, SessionReport, Ticket, load_or_create_key,
};
use stitch_node::api::{Reply, Request, SecretOp};
use stitch_node::chat::{ChatContext, transcript};
use stitch_node::client::Client;
use stitch_node::config::Config;
use stitch_node::keys::{self, Store};
use stitch_node::place::{self, Place};
use stitch_records::{Attachment, Author, Draft, NodeName, RecordId, Ref, kinds};
use stitch_store::sync::sync_roots;
use stitch_store::{AssignmentStatus, AttentionItem, BlobStore, Index, RecordRoot, Role};

mod doomed;
mod noderun;
mod service;
mod tui;
mod upgrade;
mod vault;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(
  name = "stitch",
  about = "Stitch: chats and agents on every machine you own",
  version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("STITCH_BUILD"), ")")
)]
struct Cli {
  /// The node's data directory.
  #[arg(long, global = true, env = "STITCH_DATA")]
  data: Option<PathBuf>,
  /// The record root for record commands (default: the home workspace).
  #[arg(long, global = true, env = "STITCH_ROOT")]
  root: Option<PathBuf>,
  /// Overrides the node name from config.toml.
  #[arg(long, global = true, env = "STITCH_NODE")]
  node: Option<String>,
  /// Without a command, the TUI opens.
  #[command(subcommand)]
  command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
  /// Name this node and create its home workspace.
  Setup {
    #[arg(long)]
    name: String,
    /// Where the home workspace lives (default: `home` in the data directory).
    #[arg(long)]
    home: Option<PathBuf>,
    /// Do not create a home workspace (a node that will join another's).
    #[arg(long)]
    no_home: bool,
    /// Direct addresses only: no relays, nothing published to discovery.
    #[arg(long)]
    local: bool,
  },
  /// Run the node in the foreground, or control the running one.
  Node {
    #[command(subcommand)]
    action: Option<NodeCommand>,
    /// A restarting node's successor: start once the node of process <PID>
    /// lets go of the data directory (60 s at most); not when another node
    /// took it meanwhile.
    #[arg(long, hide = true)]
    after: Option<u32>,
  },
  /// The node's shell sessions.
  Shell {
    #[command(subcommand)]
    action: ShellCommand,
  },
  /// Send a message: a new thread, or one given with --thread.
  Chat {
    /// The message; `-` or nothing reads it from standard input.
    text: Option<String>,
    /// A thread ID, or `last` (the latest here, else the latest global one).
    #[arg(long, short)]
    thread: Option<String>,
    /// A new thread that belongs to no folder.
    #[arg(long, conflicts_with = "thread")]
    global: bool,
    /// A file to attach, repeatable.
    #[arg(long = "attach", short)]
    attachments: Vec<PathBuf>,
    /// `provider/model`, e.g. `anthropic/claude-opus-5-5`.
    #[arg(long, short)]
    model: Option<String>,
    /// The repository a new thread lives in (default: the project whose
    /// checkout this is, else the home workspace).
    #[arg(long)]
    repo: Option<String>,
    /// Do not show the model's reasoning summaries.
    #[arg(long, short)]
    quiet: bool,
    /// Run it on its own (a detached thread toward this goal); its report
    /// goes to --parent, or to the thread whose shell runs this command.
    #[arg(long)]
    detach: bool,
    #[arg(long, env = "STITCH_THREAD")]
    parent: Option<String>,
    /// Budget for a detached thread: dollars.
    #[arg(long, requires = "detach")]
    budget_usd: Option<f64>,
    /// Budget for a detached thread: tokens.
    #[arg(long, requires = "detach")]
    budget_tokens: Option<u64>,
    /// Budget for a detached thread: minutes.
    #[arg(long, requires = "detach")]
    minutes: Option<u64>,
    /// Model calls each turn of the detached thread takes at most (default:
    /// the node's `[detached] max_steps`, else 50).
    #[arg(long, requires = "detach")]
    max_steps: Option<u64>,
    /// Run the detached thread on another of the user's nodes (delegation).
    #[arg(long, requires = "detach")]
    on: Option<String>,
    /// The conversation the detached thread starts from: `this`
    /// (--parent's so far, the default when given bare), `none`, or a
    /// thread's ID.
    #[arg(long, requires = "detach", num_args = 0..=1, default_missing_value = "this")]
    context: Option<String>,
    /// The detached thread runs without tools (a question to a model).
    #[arg(long, requires = "detach")]
    no_tools: bool,
    /// Its report is kept for the user on --parent's thread (shown when it
    /// is opened) and never wakes it.
    #[arg(long, requires = "detach")]
    no_wake: bool,
    /// Memory remembers the detached thread (by default it is out of
    /// memory from its start: nothing of it is remembered, though it may
    /// recall what other conversations left).
    #[arg(long, requires = "detach")]
    memory: bool,
    /// The detached thread is read-only: commands and calls the rules do
    /// not allow are refused, never asked, so it never waits on you.
    #[arg(long, requires = "detach")]
    read_only: bool,
    /// Changes to the thread's tools, comma-separated: `+github` (an MCP
    /// server), `-web` (a built-in group), `+github__get_*` (a pattern).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    tools: Vec<String>,
    /// Changes to the thread's skills, comma-separated: `-release`.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    skills: Vec<String>,
    /// A thread or effort to attach to the message, repeatable:
    /// `effort:<id>` (a conversation and every thread spawned under it),
    /// `thread:<id>` or a thread's ID. The model is briefed on it (design
    /// §2.27); `stitch scope` shows what it would be told. With
    /// `--detach`, attached to its goal (as the `spawn` tool's `scopes`).
    #[arg(long = "scope")]
    scopes: Vec<String>,
  },
  /// What attaching threads or efforts to a message tells the model (the
  /// briefing `--scope` writes with it), and with `--search`, what `recall`
  /// with that scope finds by keyword in every thread of it.
  Scope {
    /// `effort:<id>`, `thread:<id>` or a thread's ID; several together.
    #[arg(required = true)]
    targets: Vec<String>,
    #[arg(long)]
    search: Option<String>,
  },
  /// Answer an approval a thread is waiting on (see `stitch attention`).
  Answer {
    question: String,
    /// `y` allows, `n` declines, `always` allows and lets what the question
    /// grants (shown by `stitch attention`) run without asking from then on
    /// where its thread works.
    decision: String,
    /// Why, when declining.
    reason: Vec<String>,
  },
  /// Stop a running detached thread.
  Cancel { thread: String },
  /// Schedules: Stitch waking on its own at times, continuing a detached
  /// thread at each firing (design §2.3).
  Schedule {
    #[command(subcommand)]
    action: ScheduleCommand,
  },
  /// Events from outside programs (docs/events.md): publish one, list
  /// them, subscribe a thread or a schedule to a topic pattern.
  Event {
    #[command(subcommand)]
    action: EventCommand,
  },
  /// Name a thread (its title, on every node).
  Rename {
    thread: String,
    #[arg(required = true)]
    title: Vec<String>,
  },
  /// Pin a thread: first in lists, on every node (`--off` unpins).
  Pin {
    thread: String,
    #[arg(long)]
    off: bool,
  },
  /// What a conversation touched (paths read and written, URLs read,
  /// commits written, threads spawned and told), from its tool calls, the
  /// latest first; the threads of its compaction chain together.
  Artefacts { thread: String },
  /// A conversation's digest: where it stands, what it decided, found and
  /// changed, what it left open and asks of you, each item with the message
  /// it comes from. `--update` runs one update now (a model call, priced;
  /// the day's cap holds) and says what it cost. With no thread (or
  /// `--status`): how digests stand on this node: conversations with a
  /// fresh digest, a stale one, none; the day's cost against the cap; the
  /// backfill sweep. `--sweep` runs one sweep now.
  Digest {
    thread: Option<String>,
    #[arg(long)]
    update: bool,
    #[arg(long)]
    status: bool,
    #[arg(long)]
    sweep: bool,
  },
  /// Every conversation that touched a path, URL, commit or thread, in
  /// time order. The kind is guessed from the value (`--kind` says it).
  Touched {
    value: String,
    /// `path`, `url`, `commit` or `thread`.
    #[arg(long)]
    kind: Option<String>,
  },
  /// Your own instructions for the place you are in (this project or
  /// folder), or for every thread (`--global`): synced to every node, told
  /// to threads before the checkout's AGENTS.md. Shown, or edited.
  Instructions {
    /// Every thread's, not this place's.
    #[arg(long)]
    global: bool,
    /// Edit them in $EDITOR.
    #[arg(long, conflicts_with = "set")]
    edit: bool,
    /// Set them from a file (`-`: standard input; empty: none).
    #[arg(long)]
    set: Option<PathBuf>,
  },
  /// Search every thread's messages: by their words, and by their meaning
  /// where memory runs (EmbeddingGemma 2); best first.
  Search {
    /// What to look for.
    #[arg(required = true)]
    query: Vec<String>,
    /// Only the threads of the place you are in (this project or folder).
    #[arg(long)]
    here: bool,
    /// How many to show.
    #[arg(short = 'n', long, default_value_t = 20)]
    limit: usize,
  },
  /// Fork a thread: a new one continuing it from a message (read from it,
  /// not copied), with or without its attachments and tool calls, all of
  /// it or its last messages; `--summarize` puts a summary in place of
  /// what is not kept. Prints the new thread; continue it with
  /// `stitch chat -t <id>`.
  Fork {
    /// The thread, or `last` (the latest here).
    thread: String,
    /// Up to this message (`m12`, as `stitch thread` shows them).
    #[arg(long)]
    upto: Option<String>,
    /// Only its last N messages (from one of yours).
    #[arg(long)]
    last: Option<usize>,
    #[arg(long)]
    no_attachments: bool,
    /// What was said only: no tool calls or results.
    #[arg(long)]
    no_tools: bool,
    /// A summary stands for what is not kept (all of it, or what comes
    /// before `--last`).
    #[arg(long)]
    summarize: bool,
  },
  /// What runs in the background: shell commands on this node (with the
  /// job that tells their thread when they end) and detached threads on
  /// every node. `stop <id>` stops one: Ctrl+C to a command (again: its
  /// session closed), a thread cancelled.
  Jobs {
    #[arg(value_parser = ["stop"])]
    action: Option<String>,
    id: Option<String>,
  },
  /// Check the MCP servers in config.toml: start each, list its tools
  /// (those threads here get marked), stop it again.
  Mcp,
  /// Decisions (design §2.29): closed questions answered by the decision
  /// model with calibrated probabilities, for the guard, the landing gate,
  /// the triage, loose ends and agents' own (`decide` tool). Alone: the
  /// profiles, their thresholds and fallbacks, and the node's day.
  Decide {
    #[command(subcommand)]
    command: Option<DecideCommand>,
  },
  /// Shell commands run without asking (`on`: `stitch mode yolo`), or ask
  /// again (`off`: `stitch mode ask`); alone, the mode. Saved in
  /// config.toml.
  Yolo {
    #[arg(value_parser = ["on", "off"])]
    state: Option<String>,
  },
  /// The approval mode (design §2.28): `ask` (sandboxed where the system
  /// can, what reaches beyond asks; else what the rules do not allow asks),
  /// `auto` (yolo with the hazard guard: what could do severe damage that
  /// cannot be undone asks, and without the decision model's verdict a
  /// command does not run) or `yolo` (everything runs, unjudged); alone,
  /// the mode. Saved in config.toml. A place whose `.stitch/settings.toml`
  /// says `approval = "…"` follows that instead: run inside one, it says
  /// so.
  Mode {
    #[arg(value_parser = ["ask", "auto", "yolo"])]
    mode: Option<String>,
  },
  /// Continue a thread with a message, on the node that owns it (a
  /// detached thread's follow-up, any thread on another machine).
  Send {
    thread: String,
    /// The message; standard input when omitted or `-`.
    text: Option<String>,
    /// The thread its reply is reported to (and that is woken with it).
    #[arg(long, env = "STITCH_THREAD")]
    from: Option<String>,
    /// The reply is kept for the user on --from's thread and never wakes it.
    #[arg(long)]
    no_wake: bool,
    /// Weave the message into the turn running on the thread, between its
    /// steps, instead of waiting for it to end; its last reply is the
    /// reply. With no turn running it starts one.
    #[arg(long)]
    interject: bool,
    /// A thread or effort to attach to the message, repeatable (as for
    /// `chat`).
    #[arg(long = "scope")]
    scopes: Vec<String>,
  },
  /// Rooms (design §2.27): a conversation with several participants, each
  /// a thread of its own (fresh models, read-only by default, or existing
  /// threads). Talk in one with `stitch chat --thread <room>` or `stitch
  /// send <room>`; `@handle` picks who answers.
  Room {
    #[command(subcommand)]
    action: RoomCommand,
  },
  /// Harness issues: bugs in Stitch itself, filed by agents
  /// (`report_issue`) or you, and the fixes `self_improve` starts
  /// (docs/self-improve.md).
  Issue {
    #[command(subcommand)]
    action: IssueCommand,
    /// The thread asking (default: the thread whose shell runs this).
    #[arg(long, env = "STITCH_THREAD", global = true, hide = true)]
    thread: Option<String>,
  },
  /// Send a notification to your phones now (what an agent's `notify`
  /// does): from a script, a schedule's check, a long build's end.
  Notify {
    /// What it says; standard input when omitted or `-`.
    body: Option<String>,
    /// A few words above it (default: Stitch).
    #[arg(long, short)]
    title: Option<String>,
    /// The thread it is about: opening it opens that chat (default: the
    /// thread whose shell runs this command).
    #[arg(long, env = "STITCH_THREAD")]
    thread: Option<String>,
  },
  /// List chat threads: this folder's (or project's), then global ones,
  /// most recent first.
  Threads {
    /// Every thread, other folders' and projects' too.
    #[arg(long)]
    all: bool,
  },
  /// Print a thread's transcript (`last`: the latest here), switch it
  /// between read-only and writable, or show and change its tools and
  /// skills.
  #[command(args_conflicts_with_subcommands = true)]
  Thread {
    id: Option<String>,
    /// Make the thread read-only (from its next tool call): its shells
    /// write only temporary directories, only read-only commands run.
    /// Prints the mode.
    #[arg(long, conflicts_with = "writable")]
    read_only: bool,
    /// Make the thread writable (from its next turn). Prints the mode.
    #[arg(long)]
    writable: bool,
    #[command(subcommand)]
    action: Option<ThreadCommand>,
  },
  /// Provider API keys: secrets by the provider's name (`stitch secret`).
  #[command(subcommand)]
  Key(KeyCommand),
  /// Secrets (design §2.19): kept in the keychain, synced sealed to your
  /// other nodes unless this device's only, given to shells by `[env]`,
  /// never shown to a model.
  #[command(subcommand)]
  Secret(SecretCommand),
  /// The node's opt-in end-to-end encrypted vault: backup and store-and-forward
  /// (docs/vault-protocol.md; experimental, no hosted service yet).
  Vault {
    #[command(subcommand)]
    action: vault::VaultCommand,
  },
  /// A key-holding proxy for model APIs: clients (a bot's node in a
  /// container) get tokens of their own, with budgets, and never the key.
  Proxy {
    /// Where its configuration, usage and log are (default: `proxy` in
    /// the data directory).
    #[arg(long, global = true, env = "STITCH_PROXY_DIR")]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    action: Option<ProxyCommand>,
  },
  /// A Discord bot in front of this node (`DISCORD_TOKEN` or
  /// `DISCORD_TOKEN_FILE`): a mention or `/ask` in an allowed guild starts
  /// a thread, answered in a Discord thread.
  Discord {
    /// The guild IDs it answers in (commas or spaces); none: nowhere.
    #[arg(long, env = "STITCH_DISCORD_GUILDS", default_value = "")]
    guilds: String,
    /// Turns each Discord user may start an hour.
    #[arg(long, env = "STITCH_DISCORD_TURNS_PER_HOUR", default_value_t = 20)]
    turns_per_hour: usize,
    /// `provider/model`; the node's default when unset.
    #[arg(long, env = "STITCH_DISCORD_MODEL")]
    model: Option<String>,
    /// Say so in each allowed guild's system channel when connected.
    #[arg(long, env = "STITCH_DISCORD_ANNOUNCE")]
    announce: bool,
    /// A directory of per-server instructions, `<guild id>.md`, given in
    /// the first message of each new thread in that server.
    #[arg(long, env = "STITCH_DISCORD_SERVERS")]
    servers: Option<PathBuf>,
    /// Discord user IDs (commas or spaces) whose messages the agent sees
    /// tagged `[approver]`, a tag no one else can produce.
    #[arg(long, env = "STITCH_DISCORD_APPROVERS", default_value = "")]
    approvers: String,
  },
  /// Variables: values that are not secret, synced as records, given to
  /// shells by their names.
  #[command(subcommand)]
  Var(VarCommand),
  /// Models, ranked as agents see them, and your settings for them
  /// (reserved, tier, price, note), synced to every node.
  #[command(subcommand)]
  Model(ModelCommand),
  /// SSH hosts (design §2.20): records every node knows, the shell tools'
  /// `host`; their host keys; this device's key and authorizing it.
  #[command(subcommand)]
  Ssh(SshCommand),
  /// Subscription sign-in (Anthropic, OpenAI/Codex, Grok).
  #[command(subcommand)]
  Auth(AuthCommand),
  /// Push notifications to your phones, sent by this node through Apple's
  /// push service with your own key.
  #[command(subcommand)]
  Apns(ApnsCommand),
  /// Apps agents made (design §2.10): pinned ones, opened in the browser.
  #[command(subcommand)]
  App(AppCommand),
  /// The dashboard: your threads on every node, what needs you, jobs,
  /// live; pinned (with the phone too) and opened in the browser.
  Dashboard,
  /// Open the web client (the node's own UI for a browser: conversations,
  /// home, approvals) and print its address.
  Client {
    /// Print the address only; open no browser.
    #[arg(long)]
    no_open: bool,
    /// Open on this conversation (`#/t/<id>`).
    #[arg(long, conflicts_with = "map")]
    thread: Option<String>,
    /// Open on the map of conversations (`#/map`).
    #[arg(long)]
    map: bool,
  },
  /// Loose ends: the conversations that wait on you. Judges first (the
  /// decision model, the quick model where it is unsure, on those not
  /// judged at their latest step), then lists; the node does it hourly by
  /// itself (`[loose_ends]`).
  LooseEnds {
    /// Only list, judging nothing.
    #[arg(long)]
    list: bool,
    /// Hide this thread's loose end until it moves again.
    #[arg(long)]
    dismiss: Option<String>,
    /// Auto-handling: `off`, `dry-run` (records what it would do) or `on`
    /// (continues or dismisses as your rules say); synced to every node.
    #[arg(long, value_parser = ["off", "dry-run", "on"])]
    auto: Option<String>,
    /// Your rules for auto-handling: `show`, `edit` (in $EDITOR), `clear`,
    /// or a file to set them from (`-`: standard input).
    #[arg(long, value_name = "show|edit|clear|FILE")]
    rules: Option<String>,
  },
  /// Show a pairing ticket (QR code and text) for a new node.
  Invite {
    /// Minutes the ticket stays valid.
    #[arg(long, default_value_t = 10)]
    minutes: u64,
  },
  /// Join the user's nodes with a ticket from `stitch invite`.
  Join {
    ticket: String,
    /// Where to hold the home workspace (default: `home` in the data
    /// directory).
    #[arg(long)]
    home: Option<PathBuf>,
  },
  /// Sync with every known node now, or with one.
  Sync { peer: Option<String> },
  /// Print this node's endpoint ID.
  Id,
  /// The nodes trusted by this one.
  #[command(subcommand)]
  Peer(PeerCommand),
  /// Projects: every git checkout is one, known by its first commit.
  #[command(subcommand)]
  Project(ProjectCommand),
  /// The user's own Chrome through Stitch's extension, and the headless
  /// Chromium the web tools drive.
  #[command(subcommand)]
  Browser(BrowserCommand),
  /// The native messaging host Chrome starts for Stitch's extension (not
  /// for typing: `stitch browser install` sets it up).
  #[command(name = "browser-host", hide = true)]
  BrowserHost {
    /// What Chrome passes: the extension's origin (and a window handle on
    /// Windows).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
  },
  /// Memory: what was said and seen, recalled across conversations.
  #[command(subcommand)]
  Memory(MemoryCommand),
  /// Folders outside git: the same `~` path is the same folder on every
  /// node; these join or part others.
  #[command(subcommand)]
  Folder(FolderCommand),
  /// Write a new record and print its ID.
  New {
    kind: String,
    /// Becomes the body's first heading.
    #[arg(long)]
    title: Option<String>,
    /// The body; `-` reads it from standard input.
    #[arg(long)]
    body: Option<String>,
    /// `rel=[repo:]id[#sub]`, repeatable.
    #[arg(long = "ref")]
    refs: Vec<String>,
    /// `key=value`, the value in TOML (a bare word is a string), repeatable.
    #[arg(long = "field")]
    fields: Vec<String>,
    /// A file to put in the blob store and attach, repeatable.
    #[arg(long = "attach")]
    attachments: Vec<PathBuf>,
    /// Written by the user rather than by the node.
    #[arg(long)]
    user: bool,
  },
  /// Print a record and its derived status.
  Show { id: String },
  /// List records, oldest first.
  Ls {
    #[arg(long)]
    kind: Option<String>,
  },
  /// What is blocked on the user.
  Attention,
  /// Every edge of the graph, as `src rel dst`.
  Edges,
  /// Reconcile the root with another record root on this machine.
  SyncLocal { other: PathBuf },
  /// Drop the index and rebuild it from the home workspace and the projects.
  Reindex,
}

#[derive(Subcommand)]
enum RoomCommand {
  /// A new room with these participants: models (`provider/model`,
  /// `default`, `quick`, `model@node` to run it there) or threads
  /// (`thread:<id>`).
  New {
    #[arg(required = true)]
    participants: Vec<String>,
    #[arg(long)]
    title: Option<String>,
    /// Its place (`global`, `project:<repo>`, `folder:<folder>`).
    #[arg(long)]
    place: Option<String>,
    /// Threads or efforts its participants are briefed on (as `--scope`).
    #[arg(long = "scope")]
    scopes: Vec<String>,
    #[command(flatten)]
    opts: RoomOpts,
  },
  /// "Pull in model…": a room pulled out of a 1:1 thread, which joins it
  /// untouched, with these models briefed on it.
  Pull {
    thread: String,
    #[arg(required = true)]
    models: Vec<String>,
    #[command(flatten)]
    opts: RoomOpts,
  },
  /// A room's participants and policy.
  Show { room: String },
  /// Changes a room.
  Set {
    room: String,
    /// Participants to add (as for `new`), repeatable.
    #[arg(long)]
    add: Vec<String>,
    /// Participants to remove, by handle.
    #[arg(long)]
    remove: Vec<String>,
    #[arg(long)]
    mute: Vec<String>,
    #[arg(long)]
    unmute: Vec<String>,
    /// `sequential` or `parallel` (council).
    #[arg(long)]
    mode: Option<String>,
    /// How many `@handoff`s between participants a message allows.
    #[arg(long)]
    hop_cap: Option<u32>,
    /// Models added may write (not read-only).
    #[arg(long)]
    writable: bool,
    /// Models added have no tools.
    #[arg(long)]
    no_tools: bool,
  },
}

#[derive(clap::Args)]
struct RoomOpts {
  /// `sequential` (the default) or `parallel` (council).
  #[arg(long)]
  mode: Option<String>,
  #[arg(long)]
  hop_cap: Option<u32>,
  /// Dollars its participants may spend on its turns, together.
  #[arg(long)]
  budget_usd: Option<f64>,
  /// Models added may write (not read-only).
  #[arg(long)]
  writable: bool,
  /// Models added have no tools.
  #[arg(long)]
  no_tools: bool,
}

/// A participant as typed: `thread:<id>`, else a model.
fn room_participant(
  arg: &str,
  writable: bool,
  no_tools: bool,
) -> stitch_node::api::RoomParticipantSpec {
  let (thread, model) = match arg.strip_prefix("thread:") {
    Some(t) => (Some(t.to_owned()), None),
    None => (None, Some(arg.to_owned())),
  };
  stitch_node::api::RoomParticipantSpec {
    model,
    thread,
    name: None,
    slug: None,
    node: None,
    tools: !no_tools,
    read_only: !writable,
  }
}

/// A room as `RoomInfo` says it, in lines.
fn print_room(line: &str) {
  let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
    println!("{line}");
    return;
  };
  let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
  println!("room {} \"{}\"", s("room"), s("title"));
  println!(
    "  {} (hop cap {}){}{}",
    s("mode"),
    v["hop_cap"],
    if v["running"] == true {
      ", a round runs"
    } else {
      ""
    },
    v["promoted_from"]
      .as_str()
      .map(|t| format!(", pulled from {t}"))
      .unwrap_or_default()
  );
  for p in v["participants"].as_array().into_iter().flatten() {
    println!(
      "  @{:<12} {:<16} {:<36} {}{}",
      p["slug"].as_str().unwrap_or(""),
      p["name"].as_str().unwrap_or(""),
      p["model"].as_str().unwrap_or("default"),
      p["thread"].as_str().unwrap_or(""),
      if p["muted"] == true { " (muted)" } else { "" }
    );
  }
}

// Parsed once per run: the size of `Add` costs nothing.
#[derive(Clone, Subcommand)]
enum DecideCommand {
  /// Every profile: its questions, thresholds (yours over the defaults),
  /// fallback, model; the node's day.
  Profiles,
  /// Change a profile's thresholds or fallback, for every node:
  /// `guard.user --threshold 0.6` (its one question, or one for every
  /// question) or `--threshold key=0.6`. Without a profile: `--model m`,
  /// the decision model, and `--fallback <model|none>`, the model that
  /// answers (uncalibrated) when the decision model cannot (none by
  /// default: the call fails; never for guard.fixer and land, which ask).
  Set {
    /// A profile's name (`stitch decide profiles`); none for every
    /// profile's model and fallback.
    profile: Option<String>,
    /// `0.6` or `key=0.6` (a choice's option: `verdict:done=0.9`);
    /// repeatable.
    #[arg(long = "threshold")]
    thresholds: Vec<String>,
    /// The fallback model (`provider/model`, `quick`) or `none`.
    #[arg(long)]
    fallback: Option<String>,
    /// The decision model, `provider/model` (without a profile).
    #[arg(long)]
    model: Option<String>,
  },
  /// Ask a profile's questions of a text (`-`: stdin) and print the
  /// answers; `--yes-no "…?"` (repeatable) asks your own instead
  /// (the `tool` profile).
  Try {
    /// A profile's name (`tool` for your own questions).
    profile: String,
    /// The text, or `-` for stdin.
    input: String,
    /// A yes/no question of your own (repeatable), instead of the
    /// profile's.
    #[arg(long = "yes-no")]
    yes_no: Vec<String>,
  },
  /// The latest decisions this node recorded (newest last).
  Log {
    /// Only this profile's.
    #[arg(long)]
    profile: Option<String>,
    /// How many, the latest.
    #[arg(short, default_value_t = 20)]
    n: u64,
  },
}

#[derive(Subcommand)]
enum IssueCommand {
  /// The open issues and those being fixed, newest first.
  List {
    /// Fixed and dropped ones too.
    #[arg(long)]
    all: bool,
  },
  /// One issue: what happened, where, its state and fix.
  Show { issue: String },
  /// File one.
  Report {
    /// What is wrong, in a line.
    title: String,
    /// What happened.
    #[arg(long)]
    what: String,
    /// What should have.
    #[arg(long)]
    expected: Option<String>,
    #[arg(long)]
    evidence: Option<String>,
    #[arg(long)]
    area: Option<String>,
  },
  /// Change its state (open, fixing, fixed, dropped), note, fix or
  /// repository.
  Set {
    issue: String,
    state: Option<String>,
    #[arg(long)]
    note: Option<String>,
    /// The commit that fixed it.
    #[arg(long)]
    fix: Option<String>,
    /// The repository it is fixed in (`stitch issue settings` lists them).
    #[arg(long)]
    repo: Option<String>,
  },
  /// A fixer's: the node lands its commit on main for it (a fast-forward,
  /// once CI passed on that commit; touching a protected path, once the
  /// user approved what the node shows them of the diff), and sets the
  /// issue fixed.
  Land {
    issue: String,
    commit: String,
    /// What the fix changes, in 1-3 lines: shown to the user as the
    /// fixer's words, beside the node's own facts.
    #[arg(long)]
    summary: Option<String>,
    /// The repository (default: the commit's, else the one this
    /// directory is a checkout or worktree of).
    #[arg(long)]
    repo: Option<String>,
  },
  /// A fixer's: the node pushes its branches for it (it reads no
  /// credentials): a bundle of them is made here (`git bundle create`, in
  /// this checkout), and the node pushes those that are the issue's own
  /// (`fix-<the issue ID's last part>`, alone or `-<name>`) from its mirror
  /// of origin, forced.
  Push {
    issue: String,
    /// The branches (default: the one checked out).
    branches: Vec<String>,
    /// The repository (default: the one this directory is a checkout or
    /// worktree of).
    #[arg(long)]
    repo: Option<String>,
  },
  /// CI's runs on a commit (or a branch as pushed), as the node reads them.
  Ci {
    issue: String,
    commit: String,
    /// The repository (default: the commit's, else this directory's).
    #[arg(long)]
    repo: Option<String>,
  },
  /// Fetches origin into the node's mirrors (one per repository), and says
  /// how to fetch from them (a fixer reads no credentials).
  Fetch {
    /// Only this repository's.
    #[arg(long)]
    repo: Option<String>,
  },
  /// Revert an issue's landed fix on main (the node does it in its mirror,
  /// as a fast-forward) and open the issue again: yours; asks you when an
  /// agent runs it.
  Revert { issue: String },
  /// Start a fixer for it on the fixer node (`self_improve`).
  Fix {
    /// An issue's ID, or a description (filed first).
    issue: String,
    /// none, canary or fleet (default).
    #[arg(long)]
    rollout: Option<String>,
    #[arg(long)]
    budget_usd: Option<f64>,
    #[arg(long)]
    model: Option<String>,
    /// The repository it is fixed in (default: the issue's, else decided
    /// from it).
    #[arg(long)]
    repo: Option<String>,
  },
  /// Create the harness-issues schedule from the settings, pinned to the
  /// fixer node.
  Schedule,
  /// Self-improvement's settings, synced to every node: show them, or
  /// change them (the fixer node, budgets, on or off). Asks when an agent
  /// runs it.
  Settings {
    /// The fixer node: where fixes run.
    #[arg(long)]
    node: Option<String>,
    /// The Stitch checkout there (default ~/devel/stitch).
    #[arg(long)]
    place: Option<String>,
    /// Each fix's budget, in dollars.
    #[arg(long)]
    budget: Option<f64>,
    /// All fixes (and their triage) in a day, UTC, in dollars.
    #[arg(long)]
    daily: Option<f64>,
    /// The harness-issues schedule's cadence (cron).
    #[arg(long)]
    every: Option<String>,
    /// The fixers' model (`provider/model`).
    #[arg(long)]
    model: Option<String>,
    #[arg(long, conflicts_with = "off")]
    on: bool,
    #[arg(long)]
    off: bool,
    /// Fixers' and the triage's approvals: `ask` (the default) or `auto`
    /// (the hazard guard lets what it clears run in their sandbox; without
    /// its verdict they ask, never fail: design §2.28).
    #[arg(long, value_parser = ["ask", "auto"])]
    mode: Option<String>,
    /// `on`: a protected fix the landing guard clears lands unasked, with
    /// a notice and the command that reverts it (default off).
    #[arg(long, value_parser = ["on", "off"])]
    auto_land: Option<String>,
    /// The repositories fixes go to, when more than the core
    /// (docs/self-improve.md §6): a TOML array of tables, `[{ name =
    /// "core", path = "~/src/core" }, { name = "app", path = "~/src/app",
    /// core = "core", pin = "core.pin", areas = ["ios"] }]`; `""`: the
    /// core alone.
    #[arg(long, value_name = "TOML")]
    places: Option<String>,
  },
}

#[derive(Subcommand)]
enum EventCommand {
  /// Publish an event: `stitch event publish ci/failure/test
  /// '{"job": "test-linux"}'`. With `--ask <secs>`, wait for a thread's
  /// answer (printed as JSON) until that deadline; exit 2 when none came.
  #[command(alias = "pub")]
  Publish {
    topic: String,
    /// The payload: JSON (`-` reads it from stdin; none: null).
    payload: Option<String>,
    /// An ask: the deadline in seconds.
    #[arg(long)]
    ask: Option<f64>,
    /// With `--ask`: the JSON Schema the answer must match.
    #[arg(long)]
    schema: Option<String>,
    /// With `--ask`: print the ask's id and return at once (`stitch event
    /// status` follows it).
    #[arg(long)]
    no_wait: bool,
  },
  /// An event's state (an ask's answer), waiting up to `--wait` seconds
  /// for an ask to be answered.
  Status {
    id: String,
    #[arg(long, default_value_t = 0.0)]
    wait: f64,
  },
  /// The subscriptions, and the events kept here (newest first).
  Ls {
    /// Only topics matching this pattern.
    #[arg(long)]
    topic: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: usize,
  },
  /// Subscribe a thread (woken with the events, within its own budget) or
  /// a schedule (fired by them) to a topic pattern (`*` one segment, `**`
  /// any).
  Subscribe {
    pattern: String,
    #[arg(long, conflicts_with = "schedule")]
    thread: Option<String>,
    #[arg(long)]
    schedule: Option<String>,
    /// Otter code run on each event (its payload the input), at no model
    /// cost: nothing returned drops it.
    #[arg(long, conflicts_with = "filter_file")]
    filter: Option<String>,
    /// Instead of `--filter`: a `.otr` file in the thread's directory.
    #[arg(long)]
    filter_file: Option<String>,
    /// Events within this many seconds wake once (default 2).
    #[arg(long)]
    debounce: Option<f64>,
    /// Wakes per hour at most.
    #[arg(long)]
    max_per_hour: Option<u32>,
    /// What its wakes may spend a day.
    #[arg(long)]
    daily_usd: Option<f64>,
    /// When it ends: RFC 3339, `YYYY-MM-DD HH:MM`, or `in 2h`.
    #[arg(long)]
    until: Option<String>,
    /// What each wake tells the thread.
    #[arg(long)]
    note: Option<String>,
    #[command(flatten)]
    profile: ProfileArgs,
    /// With `--thread`: the thread answers the asks this subscription
    /// delivers (`event_reply`) even when it is read-only.
    #[arg(long, requires = "thread")]
    answers: bool,
  },
  /// End a subscription (a schedule's event trigger comes off it).
  Unsubscribe { subscription: String },
  /// Lift the taint untrusted events left on a thread (its capability
  /// profiles): its next turn has its own tools again.
  Untaint { thread: String },
}

/// A capability profile (docs/events.md): what the turns its events start
/// may use, enforced by the node. Any of these makes one; untrusted unless
/// `--trusted`.
#[derive(clap::Args, Clone, Default)]
struct ProfileArgs {
  /// Allowed tools, comma-separated: groups (`viewer`), names (`notify`),
  /// patterns (`support_*`); `event_reply` is always allowed.
  #[arg(long, value_delimiter = ',')]
  tools: Vec<String>,
  /// Otter shard entries, comma-separated, the last matching deciding:
  /// `+Http.Get`, `-FS.*`.
  #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
  shards: Vec<String>,
  /// Untrusted: effect classes a shard may have without being named
  /// (`io,time,suspends,random,unknown`; default none: pure shards only).
  #[arg(long, value_delimiter = ',')]
  effects: Vec<String>,
  /// A profile with nothing but `event_reply` (plus what `--tools` adds).
  #[arg(long, conflicts_with = "trusted")]
  untrusted: bool,
  /// The events are your own programs': no taint, and `--shards` only
  /// denies.
  #[arg(long)]
  trusted: bool,
}

impl ProfileArgs {
  /// The profile as requests take it, when one is asked for.
  fn value(&self) -> Option<serde_json::Value> {
    let any = self.untrusted
      || self.trusted
      || !self.tools.is_empty()
      || !self.shards.is_empty()
      || !self.effects.is_empty();
    any.then(|| {
      serde_json::json!({
        "tools": self.tools, "shards": self.shards, "effects": self.effects,
        "trusted": self.trusted,
      })
    })
  }
}

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum ScheduleCommand {
  /// Make one, in the place you are in (or `--place`), pinned to this
  /// node (or `--on <node>`): `stitch schedule add --every "0 9 * * 1-5" "Triage
  /// yesterday's CI failures"`.
  Add {
    /// What each firing asks its thread to do.
    #[arg(required = true)]
    goal: Vec<String>,
    #[arg(long)]
    name: Option<String>,
    /// Once: RFC 3339, `YYYY-MM-DD HH:MM` (in `--tz`), or `in 2h`.
    #[arg(long, conflicts_with = "every")]
    at: Option<String>,
    /// Recurring: a five-field cron expression, `@daily` and the like, or
    /// `@every 30m`.
    #[arg(long)]
    every: Option<String>,
    /// The IANA zone `at` and `every` are read in (default: the node's).
    #[arg(long)]
    tz: Option<String>,
    /// `global`, `project:<repo>` or `folder:<folder>`.
    #[arg(long)]
    place: Option<String>,
    /// The node that fires it (`--node` names this one, as everywhere).
    #[arg(long = "on")]
    on: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    max_steps: Option<u64>,
    #[arg(long)]
    read_only: bool,
    /// Each firing's budget.
    #[arg(long)]
    budget_usd: Option<f64>,
    #[arg(long)]
    budget_tokens: Option<u64>,
    #[arg(long)]
    budget_minutes: Option<f64>,
    /// At most this much a day over all its firings.
    #[arg(long)]
    daily_cap_usd: Option<f64>,
    /// Otter code run first, at no model cost: nothing returned, nothing
    /// woken.
    #[arg(long, conflicts_with = "check_file")]
    check: Option<String>,
    /// Instead of `--check`: a `.otr` file of the place.
    #[arg(long)]
    check_file: Option<String>,
    /// A new thread at each firing, not one continued.
    #[arg(long)]
    fresh: bool,
    /// Skip a firing missed while the node was down (default: run it once).
    #[arg(long)]
    no_catch_up: bool,
    /// The thread its reports go to.
    #[arg(long)]
    reports_to: Option<String>,
    /// Its reports wake that thread (default: kept for you).
    #[arg(long)]
    wake: bool,
    /// An event trigger (docs/events.md): events on topics matching this
    /// pattern fire it (`ci/failure/*`), its check run on each with
    /// the payload as input; without `--at` or `--every`, events alone.
    #[arg(long)]
    on_event: Option<String>,
    /// With `--on-event`: events within this many seconds wake once.
    #[arg(long)]
    debounce: Option<f64>,
    /// With `--on-event`: wakes per hour at most.
    #[arg(long)]
    max_per_hour: Option<u32>,
    /// With `--on-event`: when the trigger ends.
    #[arg(long)]
    until: Option<String>,
    #[command(flatten)]
    profile: ProfileArgs,
  },
  /// The schedules (`--all`: the ended ones too).
  List {
    #[arg(long)]
    all: bool,
  },
  /// One schedule, with its firings and what each cost.
  Show {
    schedule: String,
  },
  /// The next firings.
  Next,
  Pause {
    schedule: String,
  },
  Resume {
    schedule: String,
  },
  /// Fire it now (on the node it is pinned to).
  Run {
    schedule: String,
  },
  /// End it.
  Rm {
    schedule: String,
  },
}

#[derive(Subcommand)]
enum ThreadCommand {
  /// A thread's tools: listed, or changed first (`+github -web
  /// +github__get_*`; groups: shell, web, viewer, weave, memory).
  Tools {
    /// A thread ID, or `last`.
    id: String,
    #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
    changes: Vec<String>,
  },
  /// A thread's skills: listed, or changed first (`-release +mail`).
  Skills {
    id: String,
    #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
    changes: Vec<String>,
  },
  /// Delete threads for good, on every node (design §2.5): each with its
  /// compaction chain, never the threads they spawned or their forks,
  /// which stay. Lists them and asks first.
  Rm {
    /// Thread IDs (`last`: the latest here).
    ids: Vec<String>,
    /// Also every thread this one spawned (not the threads they spawned).
    #[arg(long)]
    spawned_by: Option<String>,
    /// Do not ask.
    #[arg(long, short)]
    yes: bool,
  },
  /// Share a conversation as a file (design §2.31): a `.stitch` archive
  /// (a zip: the transcript, its attachments and app versions, and a
  /// read-only `index.html`) that another Stitch imports. Secret values are
  /// redacted; what else might leak (home paths, private addresses,
  /// emails, hostnames, node names) is listed and asked about first.
  Share {
    /// A thread ID, or `last`.
    id: String,
    /// The whole spawn tree the thread is in, not its conversation alone.
    #[arg(long)]
    effort: bool,
    /// `messages` (no tool calls), `calls` (each folded to one line), or
    /// `full` (inputs and outputs whole).
    #[arg(long, default_value = "calls")]
    level: String,
    /// Where to write it (default: `<title>.stitch` here).
    #[arg(long, short)]
    out: Option<PathBuf>,
    /// Write it despite the leak scan's findings, without asking.
    #[arg(long, short)]
    yes: bool,
    /// Only say what it would hold and what the scan found.
    #[arg(long)]
    dry_run: bool,
  },
  /// Import a shared conversation's `.stitch` file as new threads:
  /// verified, previewed and asked about, then written: foreign history
  /// (data, not instructions), with no tools until `stitch event untaint
  /// <id>`.
  Import {
    file: PathBuf,
    /// Do not ask.
    #[arg(long, short)]
    yes: bool,
  },
}

#[derive(Subcommand)]
enum ProjectCommand {
  /// The projects the home workspace lists, and where they are checked
  /// out here.
  Ls,
  /// Make the fork checked out here a project of its own (by its `origin`),
  /// rather than one with the project it was forked from.
  Split,
  /// Have the project of this checkout's origin follow it here, its
  /// repository re-created from a new first commit (same origin, a new
  /// history).
  Rebind {
    /// The project, by name or ID, when several have this origin.
    #[arg(long)]
    project: Option<String>,
  },
}

#[derive(Subcommand)]
enum MemoryCommand {
  /// Download EmbeddingGemma 2 into the data directory (about 310 MB, and
  /// 555 MB more for images), which turns memory on.
  Install {
    /// Remember text only: skip the vision projector.
    #[arg(long)]
    text_only: bool,
  },
  /// Whether memory runs here, and how much it holds.
  Status,
  /// The drift check: decisions by novelty band, and strengths (run it
  /// after any change of thresholds or model).
  Audit,
  /// Every unasked note's memories, tab-separated: the cue's measures,
  /// each memory's raw and centered distance, and whether it helped, was
  /// dismissed or ignored (the data to recalibrate the note gates with).
  Notes,
  /// Take a thread out of memory: it is no longer remembered, and what it
  /// held is no longer recalled anywhere (reversible).
  Off {
    /// The thread; the latest of this place when omitted.
    thread: Option<String>,
  },
  /// Bring a thread back into memory, remembering what it holds.
  On {
    /// The thread; the latest of this place when omitted.
    thread: Option<String>,
  },
}

#[derive(Subcommand)]
enum BrowserCommand {
  /// Set up Stitch's extension for your own Chrome: write its files (to
  /// load unpacked) and the native messaging host for every Chromium
  /// browser found (Chrome, Chromium, Brave, Edge). `install headless`
  /// downloads the headless Chromium for the web tools instead.
  Install {
    /// `headless`: the web tools' Chromium (about 100 MB).
    what: Option<String>,
    /// Write the host manifest into this browser user-data directory only
    /// (a test profile), not into the browsers found.
    #[arg(long)]
    profile_dir: Vec<PathBuf>,
  },
  /// The Chrome profiles that have connected, and whether Stitch is
  /// enabled in each; `disable <name>` turns one off (enabling takes a
  /// click in that profile's extension popup).
  Profiles {
    /// `disable`
    action: Option<String>,
    name: Option<String>,
  },
  /// Run a browser tool as a thread would, outside any turn (for trying
  /// things and tests): `stitch browser call browser_tabs '{"action":"new","url":"…"}'`.
  Call {
    tool: String,
    #[arg(default_value = "{}")]
    input: String,
    /// The session's owner (default: `cli`, one session shared by calls).
    #[arg(long)]
    thread: Option<String>,
    /// Where images go (default: printed paths in the downloads folder).
    #[arg(long)]
    images: Option<PathBuf>,
  },
}

#[derive(Subcommand)]
enum FolderCommand {
  /// Make this folder the same as another: `node:path`, or a path here.
  /// The other must be a folder Stitch knows (a directory here, or one a
  /// thread, a folder record or a folder sync shows on its node).
  Link {
    other: String,
    /// Link even a location Stitch does not know.
    #[arg(long)]
    force: bool,
  },
  /// Keep this folder to this node, apart from same-named folders on
  /// others and from its links.
  Unlink,
  /// Sync this folder's files with the user's other machines now (it
  /// happens before and after each turn here, and after each sync). A new
  /// folder syncs only once the user lets it: `--allow`, or the question
  /// its first thread asks; `--stop` keeps its files on each node.
  Sync {
    #[arg(long)]
    allow: bool,
    #[arg(long, conflicts_with = "allow")]
    stop: bool,
  },
  /// This folder's changes, the latest first: when, on which node, by
  /// which thread, what they changed; only a file's (or directory's) when
  /// given.
  Log {
    path: Option<String>,
    #[arg(short = 'n', long, default_value_t = 20)]
    limit: usize,
  },
  /// What a change did, as a diff: the latest by default (`--to`), against
  /// what it follows (`--from`); only a file's when given.
  Diff {
    path: Option<String>,
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    to: Option<String>,
  },
}

#[derive(Subcommand)]
enum ShellCommand {
  /// Put a session an agent drives in this terminal, live (`<thread>/<session>`
  /// as `stitch jobs` lists it): watch it, type into it (the agent is told,
  /// and sends nothing on top of it until it looked). Ctrl+] detaches.
  Attach { id: String },
}

#[derive(Subcommand)]
enum NodeCommand {
  /// Stop the running node (its shells and running turns end; detached
  /// threads resume at its next start).
  Stop {
    /// Once nothing runs instead of now (a service then takes over).
    #[arg(long)]
    when_idle: bool,
  },
  /// Restart the node once nothing runs (or `--now`): the node starts its
  /// successor itself, from the binary installed now, outside any sandbox
  /// of the shell asking. Detached threads resume.
  Restart {
    /// Now: running turns are cut and resume, shells end.
    #[arg(long)]
    now: bool,
  },
  /// The running node's name, ID, build, repositories and peers.
  Status,
  /// The builds this node keeps and their health (`<data>/bin`): the
  /// healthy one and since when, the one on trial, those that failed.
  Builds {
    /// Succeed only when the build installed is the one running, healthy
    /// for at least this many seconds (a rollout's trial).
    #[arg(long, value_name = "SECS")]
    healthy_for: Option<u64>,
  },
  /// The node as a service: a systemd user unit (Linux) or a LaunchAgent
  /// (macOS), started at login, started again when it asks to restart.
  Service {
    #[command(subcommand)]
    action: ServiceCommand,
  },
}

#[derive(Subcommand)]
enum ServiceCommand {
  /// Write, enable and start it. A node running outside it stops once
  /// idle, and the service's takes over.
  Install {
    /// Also update from main every 30 minutes (pull, build, install; the
    /// node restarts into the new build once idle), from this checkout.
    #[arg(long, value_name = "CHECKOUT", num_args = 0..=1, default_missing_value = "~/devel/stitch")]
    update: Option<String>,
    /// The rollout (with --update): follow this branch instead of main
    /// (`fleet`: what the canary promoted).
    #[arg(long, requires = "update")]
    follow: Option<String>,
    /// The rollout: push a build healthy here for 30 minutes to this
    /// branch, for the next nodes (the canary: `fleet`).
    #[arg(long, requires = "update")]
    promote: Option<String>,
    /// The rollout, macOS: upload the app to TestFlight once a new build
    /// is healthy here (the phone follows), with the app checkout's
    /// justfile (default: `--app`'s, else the checkout's own when it holds
    /// the app).
    #[arg(long, requires = "update", value_name = "APP_CHECKOUT", num_args = 0..=1, default_missing_value = "")]
    testflight: Option<String>,
    /// The rollout: the app's checkout, followed and promoted with this
    /// one (the same branch), so a commit landed in the app's repository
    /// alone reaches the Mac that builds it (docs/self-improve.md §6).
    #[arg(long, requires = "update", value_name = "APP_CHECKOUT")]
    app: Option<String>,
    /// The binary it runs (default: this one).
    #[arg(long)]
    exe: Option<PathBuf>,
    /// Only print the files it would write.
    #[arg(long)]
    print: bool,
  },
  /// Stop and remove it (and the update timer).
  Uninstall,
}

#[derive(Subcommand)]
enum AppCommand {
  /// The pinned apps, with their addresses here.
  List,
  /// Open an app in the browser: a pinned one (by name or ID), the latest
  /// one a thread showed (by the thread's ID), or a version (`blake3:…`);
  /// with none, the list of pinned apps.
  Open { app: Option<String> },
  /// Pin an app: the latest one a thread showed, or a version
  /// (`blake3:…`); pinned again from the same thread, it moves to the
  /// newer version. Pinned from a thread, it follows it: each newer
  /// version of its name the thread shows moves it. A version stays.
  Pin {
    what: String,
    /// Its name (default: the app's own).
    #[arg(long)]
    name: Option<String>,
    /// Its icon on the phone: an SF Symbol's name (`checklist`, `chart.bar`).
    #[arg(long)]
    icon: Option<String>,
  },
  /// Unpin an app (by name or ID); its versions stay in their threads.
  Unpin { app: String },
  /// Write an app's files to a directory: a pinned app (by name or ID),
  /// the latest one a thread showed (by the thread's ID), or a version
  /// (`blake3:…`).
  Export {
    what: String,
    /// Where (default: `./<its name>`, made lower-case with dashes).
    dir: Option<PathBuf>,
    /// Write into a directory that is not empty, replacing its files.
    #[arg(long)]
    force: bool,
  },
  /// Let an app see your threads, jobs and approvals (`stitch.live`): a
  /// pinned app by name or ID, or an app in its conversation by its name
  /// and `--thread`. Opened in a browser, an app has no other way to ask.
  AllowLive {
    app: String,
    /// The thread whose conversation shows the app (not pinned).
    #[arg(long)]
    thread: Option<String>,
    /// Take it back: it may no longer.
    #[arg(long)]
    deny: bool,
  },
  /// Show what an app's `stitch-app.json` asks for threads of its own
  /// (places as resolved here, tools, skills, models, budgets) and the
  /// hash of those needs; grant exactly them with `--needs <hash>`. A
  /// changed manifest asks again. A pinned app by name or ID, or an app in
  /// its conversation by its exact name and `--thread`.
  AllowThreads {
    app: String,
    /// The thread whose conversation shows the app (not pinned).
    #[arg(long)]
    thread: Option<String>,
    /// Grant the needs shown with this hash (refused when the app's
    /// latest version asks for others by now).
    #[arg(long)]
    needs: Option<String>,
    /// Take it back: nothing more of its threads runs, those at work are
    /// cancelled.
    #[arg(long, conflicts_with = "needs")]
    deny: bool,
  },
  /// Show what an app asks of containers (design §2.23): its build (the
  /// Containerfile, shown, or its change since the last build grant; the
  /// build runs the app's code with internet access), its commands, its
  /// shell, each with a hash; grant each with its hash. A changed
  /// Containerfile or tool asks again. Linux with rootless podman only.
  AllowContainer {
    app: String,
    /// The thread whose conversation shows the app (not pinned).
    #[arg(long)]
    thread: Option<String>,
    /// Grant the build shown with this hash (the build starts).
    #[arg(long)]
    build: Option<String>,
    /// Grant the commands shown with this hash.
    #[arg(long)]
    run: Option<String>,
    /// Grant the shell shown with this hash (it cancels pinning).
    #[arg(long)]
    shell: Option<String>,
    /// Take them all back; its containers running are stopped.
    #[arg(long, conflicts_with_all = ["build", "run", "shell"])]
    deny: bool,
  },
  /// Apps' container images and caches on this node, with their sizes; or
  /// one app's removed (`--rm <name>`: its images and cache, rebuilt at its
  /// next run; its data folder stays).
  Storage {
    #[arg(long = "rm")]
    remove: Option<String>,
  },
}

#[derive(Subcommand)]
enum ApnsCommand {
  /// Keep an APNs key (the `.p8` from developer.apple.com, Keys) for this
  /// node to push with: in the keychain, its IDs in config.toml.
  Set {
    /// The `AuthKey_<id>.p8` file.
    p8: PathBuf,
    /// The key's ID, as the developer site shows it.
    #[arg(long)]
    key_id: String,
    /// The team that made the key: the one that signs the app
    /// (project.yml's DEVELOPMENT_TEAM, ten characters).
    #[arg(long)]
    team_id: String,
    /// In an owner-only file instead of the keychain.
    #[arg(long)]
    file: bool,
  },
  /// Push through another relay than the app's (the Worker in relay/,
  /// the default for a node without a key of its own).
  Relay { url: String },
  /// Push a test notification to your phones.
  Test,
  /// Whether this node pushes, and the phones it would push to.
  Status,
}

#[derive(Subcommand)]
enum AuthCommand {
  /// Sign in: a subscription (`anthropic`, `openai`, `grok`: a browser
  /// sign-in URL, then the local callback), or `github` (a code to enter
  /// at github.com/login/device).
  Login {
    provider: String,
    /// Store tokens in an owner-only file instead of the keychain.
    #[arg(long)]
    file: bool,
  },
  /// Remove the local subscription session (API keys are separate).
  Logout { provider: String },
  /// Show local sign-in state without printing or refreshing tokens.
  Status { provider: Option<String> },
}

#[derive(Subcommand)]
enum KeyCommand {
  /// Store a provider's key (read without echo, or from standard input):
  /// `stitch secret set <provider>`.
  Set {
    provider: String,
    /// In `credentials.toml` instead of the keychain.
    #[arg(long)]
    file: bool,
  },
  /// Remove a provider's key: `stitch secret rm <provider>`.
  Rm { provider: String },
}

/// Where a secret or variable applies.
#[derive(clap::Args, Clone)]
struct ScopeArgs {
  /// `global` (the default), `project:<repo>` or `folder:<key>`.
  #[arg(long, default_value = "global", conflicts_with = "here")]
  scope: String,
  /// The place the current directory is in (a project or a folder).
  #[arg(long)]
  here: bool,
}

impl ScopeArgs {
  fn scope(&self) -> String {
    if self.here {
      "here".into()
    } else {
      self.scope.clone()
    }
  }
}

#[derive(Subcommand)]
enum SecretCommand {
  /// Keep a secret (read without echo, or from standard input); synced
  /// sealed to your other nodes unless `--device-only`.
  Set {
    name: String,
    #[command(flatten)]
    scope: ScopeArgs,
    /// Kept on this device only, never synced (removed from the others).
    #[arg(long, conflicts_with = "sync")]
    device_only: bool,
    /// Synced to your other nodes (a secret that was this device's only).
    #[arg(long)]
    sync: bool,
    /// Change only `--device-only` or `--sync`, keeping the value.
    #[arg(long)]
    keep_value: bool,
    /// In `credentials.toml` instead of the keychain.
    #[arg(long)]
    file: bool,
  },
  /// Whether this node holds a secret; its value with `--reveal`.
  Get {
    name: String,
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long)]
    reveal: bool,
  },
  /// The secrets: where each is held and used (never their values).
  Ls,
  /// Remove a secret here and, unless this device's only, everywhere.
  Rm {
    name: String,
    #[command(flatten)]
    scope: ScopeArgs,
  },
}

#[derive(Subcommand)]
enum ProxyCommand {
  /// Serve (the default): forward model requests with this machine's keys.
  Serve {
    /// Addresses to listen on (default: `listen` in proxy.toml, else
    /// 127.0.0.1:7178 and Docker's bridge gateway 172.17.0.1:7178).
    #[arg(long)]
    listen: Vec<String>,
  },
  /// Tokens for clients.
  #[command(subcommand)]
  Token(ProxyTokenCommand),
  /// Each token's limits, and what it and its end users used by day.
  Usage {
    #[arg(long, default_value_t = 7)]
    days: usize,
  },
}

#[derive(Subcommand)]
enum ProxyTokenCommand {
  /// Make a token (printed once; only its hash is kept), replacing one of
  /// the name.
  Add {
    name: String,
    /// Budget tokens a day (UTC), all its users together.
    #[arg(long)]
    daily_tokens: Option<u64>,
    /// Budget tokens a day for each end user it names.
    #[arg(long)]
    daily_tokens_per_user: Option<u64>,
    /// Requests a minute (default 30).
    #[arg(long)]
    rpm: Option<u32>,
    /// Upstreams it may use (default: all).
    #[arg(long = "upstream")]
    upstreams: Vec<String>,
  },
  /// The tokens and their limits.
  Ls,
  Rm {
    name: String,
  },
}

#[derive(Subcommand)]
enum VarCommand {
  Set {
    name: String,
    value: String,
    #[command(flatten)]
    scope: ScopeArgs,
  },
  Ls,
  Rm {
    name: String,
    #[command(flatten)]
    scope: ScopeArgs,
  },
}

#[derive(Subcommand)]
enum ModelCommand {
  /// The models with prices, and what you set.
  Ls {
    /// Another node's models.
    #[arg(long)]
    node: Option<String>,
  },
  /// The default model, for every node (alone: says it); a node's
  /// `config.toml` `default_model` wins on it, a project's
  /// `.stitch/settings.toml` `[models] default` in it.
  Default {
    /// `provider/model`.
    model: Option<String>,
    #[arg(long, conflicts_with = "model")]
    unset: bool,
  },
  /// The quick model agents spawn quick work on (`model: "quick"`), for
  /// every node (alone: says it); a node's `quick_model` wins on it,
  /// `[models] quick` of a project in it.
  Quick {
    /// `provider/model`.
    model: Option<String>,
    #[arg(long, conflicts_with = "model")]
    unset: bool,
  },
  /// Set what is given for models matching a pattern (`*` matches
  /// anything): `stitch model set 'anthropic/claude-fable-*' --reserved`.
  Set {
    pattern: String,
    /// Agents spawn on it only when you asked for it by name
    /// (`--reserved false` to undo).
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    reserved: Option<bool>,
    /// USD per million tokens, input/output (`2/12`); empty clears it.
    #[arg(long)]
    price: Option<String>,
    /// Shown beside it; empty clears it.
    #[arg(long)]
    note: Option<String>,
  },
  /// Remove the settings of a pattern.
  Rm { pattern: String },
}

#[derive(Subcommand)]
enum SshCommand {
  /// SSH hosts: add (or replace), list, remove.
  #[command(subcommand)]
  Host(SshHostCommand),
  /// This device's public key (made the first time), for a host's
  /// `authorized_keys`.
  Key,
  /// Add a device's public key to a host's `authorized_keys`: this
  /// node's (or `--node`'s, or `--key`), done here when this node gets in,
  /// else asked of the nodes that do.
  Authorize {
    host: String,
    /// The node whose published key to add.
    #[arg(long, conflicts_with = "key")]
    node: Option<String>,
    /// An `authorized_keys` line to add.
    #[arg(long)]
    key: Option<String>,
  },
  /// Requests to authorize keys, and how they went.
  Authorizations,
  /// Host keys trusted (on first use): list, forget.
  #[command(subcommand)]
  Hostkey(SshHostKeyCommand),
}

#[derive(Subcommand)]
enum SshHostCommand {
  /// Add a host, or replace the one of that name (a record every node
  /// knows).
  Add {
    name: String,
    /// Where it is (default: the name).
    #[arg(long)]
    address: Option<String>,
    #[arg(long)]
    port: Option<u16>,
    #[arg(long)]
    user: Option<String>,
    /// A host to go through (its name).
    #[arg(long)]
    jump: Option<String>,
    /// auto (the agent and key files, then this device's key), agent,
    /// device, secret:<name> (a password), key:<path>.
    #[arg(long, default_value = "auto")]
    auth: String,
    /// relaxed (what reaches beyond asks), ask (what is not read-only
    /// asks), allow (trusted whole).
    #[arg(long, default_value = "relaxed")]
    approval: String,
    /// Plain shells, not tmux, even where the host has it.
    #[arg(long)]
    no_tmux: bool,
  },
  Ls,
  Rm {
    name: String,
  },
}

#[derive(Subcommand)]
enum SshHostKeyCommand {
  Ls,
  /// Forget the keys of a host (its name, or `[address]:port`): the next
  /// connection trusts what it finds.
  Rm {
    host: String,
  },
}

#[derive(Subcommand)]
enum PeerCommand {
  /// Trust a node by name and endpoint ID (pairing does this for you).
  Add {
    name: String,
    id: String,
    /// A direct address (`ip:port`), repeatable; needed without discovery.
    #[arg(long = "addr")]
    addrs: Vec<std::net::SocketAddr>,
  },
  Ls,
  /// Stop trusting a node: a `revoke` record in the home workspace, which
  /// spreads to every node, and removal from the local list.
  Remove {
    name: String,
  },
}

fn data_dir(cli: &Cli) -> Result<PathBuf> {
  if let Some(d) = &cli.data {
    return Ok(d.clone());
  }
  let home = std::env::var_os("HOME").ok_or("set --data or HOME")?;
  Ok(Path::new(&home).join(".stitch"))
}

fn config(cli: &Cli) -> Result<Config> {
  Ok(Config::load(&data_dir(cli)?)?)
}

fn node_name(cli: &Cli) -> Result<NodeName> {
  match &cli.node {
    Some(n) => Ok(n.parse()?),
    None => Ok(config(cli)?.node_name()?),
  }
}

fn open_index(cli: &Cli) -> Result<Index> {
  let dir = data_dir(cli)?;
  std::fs::create_dir_all(&dir)?;
  Ok(Index::open(&dir.join("index.sqlite"))?)
}

/// The home workspace's root, from the index.
fn home_root(index: &Index) -> Result<Option<RecordRoot>> {
  for dir in index.roots()?.into_values() {
    if let Ok(r) = RecordRoot::open(Path::new(&dir))
      && r.config().role == Role::Home
    {
      return Ok(Some(r));
    }
  }
  Ok(None)
}

/// Every record root this data directory holds, indexed.
fn all_roots(index: &mut Index) -> Result<Vec<RecordRoot>> {
  let mut out = Vec::new();
  for dir in index.roots()?.into_values() {
    if let Ok(r) = RecordRoot::open(Path::new(&dir)) {
      index.refresh(&r)?;
      out.push(r);
    }
  }
  Ok(out)
}

/// The place of the directory this command runs in.
fn here(cli: &Cli, roots: &[RecordRoot], places: &place::Places) -> Result<Place> {
  Ok(here_and_checkout(cli, roots, places)?.0)
}

/// The latest thread here (`stitch thread last`).
/// `Request::Import` of a shared file's bytes (design §2.31).
async fn import_request(
  c: &mut stitch_node::client::Client,
  bundle: &[u8],
  preview: bool,
) -> Result<stitch_node::share::Imported> {
  let req = Request::Import {
    bundle: bundle.to_vec(),
    preview,
  };
  match c.request(&req).await? {
    Reply::Lines(l) if !l.is_empty() => Ok(serde_json::from_str(&l[0])?),
    Reply::Error(e) => Err(e.into()),
    other => Err(format!("unexpected reply: {other:?}").into()),
  }
}

fn last_thread(cli: &Cli) -> Result<RecordId> {
  let mut index = open_index(cli)?;
  let roots = all_roots(&mut index)?;
  let here = here_folded(cli, &mut index, &roots)?;
  let places = place::Places::load(&index);
  let home = roots
    .iter()
    .find(|r| r.config().role == Role::Home)
    .map(|r| r.repo().clone());
  Ok(
    place::ordered(
      index.listed_threads()?,
      home.as_ref(),
      &here,
      false,
      &places,
    )
    .first()
    .map(|(r, ..)| r.id.clone())
    .ok_or("no thread yet")?,
  )
}

/// The place of the directory this command runs in, after folding the
/// folder it was before `git init` into its project (see `place::fold`).
fn here_folded(cli: &Cli, index: &mut Index, roots: &[RecordRoot]) -> Result<Place> {
  let (here, co) = here_and_checkout(cli, roots, &place::Places::load(index))?;
  if let Some(home) = roots.iter().find(|r| r.config().role == Role::Home) {
    place::fold(index, home, &node_name(cli)?, &here, co.as_ref())?;
  }
  Ok(here)
}

/// The place of the directory this command runs in, with its checkout.
fn here_and_checkout(
  cli: &Cli,
  roots: &[RecordRoot],
  places: &place::Places,
) -> Result<(Place, Option<place::Checkout>)> {
  let cwd = std::env::current_dir()?;
  let home = roots.iter().find(|r| r.config().role == Role::Home);
  let here = Place::of_dir(
    &cwd,
    node_name(cli)?.as_str(),
    home.map(RecordRoot::dir),
    places,
  );
  if matches!(here.0, Place::Folder(_))
    && let Some(top) = place::unread_checkout(&cwd)
  {
    eprintln!("stitch: {}", place::unread_checkout_note(&top));
  }
  Ok(here)
}

/// The record root for record commands, indexed.
fn open(cli: &Cli) -> Result<(RecordRoot, Index)> {
  let mut index = open_index(cli)?;
  let root = match &cli.root {
    Some(r) => RecordRoot::open(r)?,
    None => home_root(&index)?.ok_or("no record root: pass --root, or run `stitch setup`")?,
  };
  let report = index.refresh(&root)?;
  for (path, why) in &report.quarantined {
    eprintln!("quarantined {path}: {why}");
  }
  Ok((root, index))
}

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime> {
  Ok(
    tokio::runtime::Builder::new_multi_thread()
      .enable_all()
      .build()?,
  )
}

/// A client of this data directory's node, started if needed.
/// Opens `url` in the user's browser; where none can be started, prints it.
fn open_in_browser(url: &str) {
  if !launch_browser(url) {
    println!("{url}");
  }
}

/// Opens `url` in the user's browser, printing nothing: whether it could.
fn launch_browser(url: &str) -> bool {
  let opener = if cfg!(target_os = "macos") {
    Some("open")
  } else if cfg!(target_os = "linux") {
    Some("xdg-open")
  } else {
    None
  };
  opener.is_some_and(|o| {
    std::process::Command::new(o)
      .arg(url)
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .status()
      .is_ok_and(|s| s.success())
  })
}

async fn client(cli: &Cli) -> Result<Client> {
  let exe = upgrade::installed_exe()?;
  Ok(Client::connect_or_start(&data_dir(cli)?, &exe).await?)
}

fn print_qr(text: &str) -> Result<()> {
  let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L)?;
  let image = code
    .render::<qrcode::render::unicode::Dense1x2>()
    .dark_color(qrcode::render::unicode::Dense1x2::Light)
    .light_color(qrcode::render::unicode::Dense1x2::Dark)
    .quiet_zone(true)
    .build();
  println!("{image}");
  Ok(())
}

fn print_report(r: &SessionReport) {
  println!(
    "{}: {} in, {} out, blobs {} in, {} out",
    r.repo,
    r.received.len(),
    r.sent,
    r.blobs_received,
    r.blobs_sent
  );
}

fn parse_field(s: &str) -> Result<(String, toml::Value)> {
  let (k, v) = s
    .split_once('=')
    .ok_or_else(|| format!("`{s}`: expected key=value"))?;
  let value = match toml::from_str::<toml::Table>(&format!("v = {v}")) {
    Ok(mut t) => t.remove("v").expect("parsed"),
    Err(_) => toml::Value::String(v.to_owned()),
  };
  Ok((k.to_owned(), value))
}

fn status(index: &Index, id: &RecordId, kind: &str) -> Result<String> {
  Ok(match kind {
    kinds::TASK => index.task_status(id)?.as_str().to_owned(),
    kinds::QUESTION => if index.question_answered(&Ref::to(id.clone()))? {
      "answered"
    } else {
      "open"
    }
    .to_owned(),
    kinds::RUN => if index.run_ended(id)? {
      "ended"
    } else {
      "active"
    }
    .to_owned(),
    kinds::ASSIGNMENT => match index.assignment_status(id, jiff::Timestamp::now())? {
      AssignmentStatus::Unclaimed => "unclaimed".to_owned(),
      AssignmentStatus::Claimed { node, until } => format!("claimed by {node} until {until}"),
      AssignmentStatus::Expired { node, until } => format!("expired ({node}, {until})"),
    },
    _ => "-".to_owned(),
  })
}

/// A secret, read without echo on a terminal, else from standard input.
fn read_secret(prompt: &str) -> Result<String> {
  let value = if std::io::stdin().is_terminal() {
    rpassword::prompt_password(prompt)?
  } else {
    read_stdin()?
  };
  let value = value.trim().to_owned();
  if value.is_empty() {
    return Err("an empty secret".into());
  }
  Ok(value)
}

/// Asks the node about secrets and variables; prints what it says in
/// lines, a revealed value as it is; the rest is the caller's.
fn secret_request(cli: &Cli, op: SecretOp) -> Result<Reply> {
  let req = Request::Secret {
    op,
    cwd: std::env::current_dir()
      .ok()
      .map(|d| d.display().to_string()),
  };
  let reply = runtime()?.block_on(async {
    let mut c = client(cli).await?;
    c.request(&req).await.map_err(Box::<dyn Error>::from)
  })?;
  match &reply {
    Reply::Lines(lines) => {
      for l in lines {
        println!("{l}");
      }
    }
    Reply::Secret(value) => println!("{value}"),
    _ => {}
  }
  Ok(reply)
}

fn ssh_command(cli: &Cli, cmd: &SshCommand) -> Result<()> {
  use stitch_node::api::{SshHost, SshOp};
  let op = match cmd {
    SshCommand::Host(SshHostCommand::Add {
      name,
      address,
      port,
      user,
      jump,
      auth,
      approval,
      no_tmux,
    }) => SshOp::HostSet(SshHost {
      name: name.clone(),
      address: address.clone().unwrap_or_default(),
      port: port.unwrap_or(0),
      user: user.clone().unwrap_or_default(),
      jump: jump.clone().unwrap_or_default(),
      auth: auth.clone(),
      approval: approval.clone(),
      no_tmux: *no_tmux,
      from: String::new(),
    }),
    SshCommand::Host(SshHostCommand::Ls) => SshOp::Hosts,
    SshCommand::Host(SshHostCommand::Rm { name }) => SshOp::HostRemove { name: name.clone() },
    SshCommand::Key => SshOp::Key,
    SshCommand::Authorize { host, node, key } => SshOp::Authorize {
      host: host.clone(),
      key: key.clone(),
      node: node.clone(),
    },
    SshCommand::Authorizations => SshOp::Authorizations,
    SshCommand::Hostkey(SshHostKeyCommand::Ls) => SshOp::HostKeys,
    SshCommand::Hostkey(SshHostKeyCommand::Rm { host }) => {
      SshOp::HostKeyRemove { host: host.clone() }
    }
  };
  let reply = runtime()?.block_on(async {
    let mut c = client(cli).await?;
    c.request(&Request::Ssh { op })
      .await
      .map_err(Box::<dyn Error>::from)
  })?;
  match reply {
    Reply::Lines(lines) => {
      for l in lines {
        println!("{l}");
      }
    }
    Reply::SshHosts(hosts) => {
      if hosts.is_empty() {
        println!("no SSH hosts: stitch ssh host add <name> --address … --user …");
      }
      for h in hosts {
        let at = if h.address.is_empty() {
          h.name.clone()
        } else {
          h.address.clone()
        };
        let mut line = format!(
          "{}  {}{at}{}",
          h.name,
          if h.user.is_empty() {
            String::new()
          } else {
            format!("{}@", h.user)
          },
          if h.port == 0 || h.port == 22 {
            String::new()
          } else {
            format!(":{}", h.port)
          }
        );
        if !h.jump.is_empty() {
          line.push_str(&format!("  via {}", h.jump));
        }
        if !h.auth.is_empty() && h.auth != "auto" {
          line.push_str(&format!("  auth {}", h.auth));
        }
        if !h.approval.is_empty() && h.approval != "relaxed" {
          line.push_str(&format!("  approval {}", h.approval));
        }
        if h.no_tmux {
          line.push_str("  no tmux");
        }
        if h.from == "ssh-config" {
          line.push_str("  (~/.ssh/config)");
        }
        println!("{line}");
      }
    }
    Reply::SshHostKeys(keys) => {
      if keys.is_empty() {
        println!("no host keys yet: they are trusted on first use");
      }
      for k in keys {
        println!(
          "{}  {}  {}  (trusted by {}, {})",
          k.host, k.name, k.fingerprint, k.node, k.trusted
        );
      }
    }
    Reply::SshKey {
      line,
      fingerprint,
      kind,
    } => {
      println!("{line}");
      eprintln!("{fingerprint} ({kind}); `stitch ssh authorize <host>` adds it to a host");
    }
    Reply::SshAuthorizations(list) => {
      if list.is_empty() {
        println!("no requests to authorize keys");
      }
      for a in list {
        println!("{}  {}  {}  ({})", a.host, a.key, a.state, a.asked);
        for s in a.said {
          println!("    {s}");
        }
      }
    }
    _ => {}
  }
  Ok(())
}

fn print_secret(s: &stitch_node::api::SecretInfo) {
  let scope = if s.scope == "global" {
    String::new()
  } else {
    format!(" ({})", s.scope_label)
  };
  let held = if s.device_only {
    "this device only".to_owned()
  } else {
    format!("on {}", s.nodes.join(", "))
  };
  let here = if s.here { "" } else { ", not held here yet" };
  println!(
    "{}{scope}: {held}{here}; set {} on {}",
    s.name, s.updated, s.from
  );
  if !s.used.is_empty() {
    println!("  used by {}", s.used.join(", "));
  }
  if !s.rotate.is_empty() {
    println!(
      "  rotate it: {} held it and was removed",
      s.rotate.join(", ")
    );
  }
}

fn read_stdin() -> Result<String> {
  let mut s = String::new();
  std::io::stdin().read_to_string(&mut s)?;
  Ok(s)
}

/// Asks on the terminal whether a command may run: `y` allows, `n` or
/// nothing declines, anything else declines with that as the reason. No
/// terminal declines.
/// `stitch mcp`: every configured server started here (not in the node),
/// its tools listed, then stopped.
async fn check_mcp(data: &Path) -> Result<()> {
  use stitch_node::mcp::{Mcp, McpTools};
  let settings = Config::load(data)?.mcp.unwrap_or_default();
  if settings.servers.is_empty() {
    println!(
      "no MCP servers: add [mcp.servers.<name>] to {}",
      data.join("config.toml").display()
    );
    return Ok(());
  }
  let hub = Mcp::new(data);
  hub.configure(&settings.servers);
  let cwd = std::env::current_dir().ok();
  let here = McpTools::new(
    &hub,
    &settings.enabled(cwd.as_deref()),
    data.join("downloads"),
  );
  here.prepare().await;
  let enabled: std::collections::BTreeSet<String> =
    here.definitions().into_iter().map(|d| d.name).collect();
  for server in hub.servers() {
    match server.tools().await {
      Ok(tools) => {
        println!(
          "{}: {} tool(s){}",
          server.name,
          tools.len(),
          if server.settings.share == Some(true) {
            ", shared with your other nodes"
          } else {
            ""
          }
        );
        for t in tools {
          let name = stitch_node::mcp::exposed_name(&server.name, &t.name)
            .unwrap_or_else(|| format!("{} (name too long: left out)", t.name));
          println!(
            "  {name}{}{}",
            if enabled.contains(&name) {
              "  [enabled here]"
            } else {
              ""
            },
            if server.allows(&t.name) {
              "  [runs without asking]"
            } else {
              ""
            }
          );
        }
      }
      Err(e) => println!("{}: {e}", server.name),
    }
  }
  hub.stop_all().await;
  Ok(())
}

/// Asks on the terminal; `None` when there is none (the approval then
/// waits on the attention list).
fn ask_approval(
  session: &str,
  command: &str,
  grant: &[String],
) -> Option<stitch_node::tools::Decision> {
  use std::io::BufRead;
  use stitch_node::tools::Decision;
  let Ok(mut tty) = std::fs::OpenOptions::new()
    .read(true)
    .write(true)
    .open("/dev/tty")
  else {
    eprintln!("[`{command}` waits for approval: `stitch attention`]");
    return None;
  };
  let asked = if session == stitch_node::tools::STEPS_SESSION {
    command.to_owned()
  } else if session == "main" {
    format!("run: {command}")
  } else {
    format!("run [{session}]: {command}")
  };
  let always = if grant.is_empty() {
    String::new()
  } else {
    let list: Vec<String> = grant.iter().map(|g| format!("`{g}`")).collect();
    format!(", a: always allow {} here", list.join(", "))
  };
  let _ = write!(
    tty,
    "\x1b[1m{asked}\x1b[0m\n  allow? [y/N{always}, or why not] "
  );
  let _ = tty.flush();
  let mut line = String::new();
  let _ = std::io::BufReader::new(tty).read_line(&mut line);
  Some(match line.trim() {
    "y" | "Y" | "yes" => Decision::allow(),
    "a" | "A" | "always" if !grant.is_empty() => Decision::always(),
    "" | "n" | "N" | "no" => Decision::decline(None),
    reason => Decision::decline(Some(reason.to_owned())),
  })
}

/// Stops the running node, if any, and waits for its lock to be free.
/// `stitch shell attach`: the terminal raw, keys to the session (Ctrl+]
/// detaches), what it shows to the terminal.
async fn attach(cli: &Cli, id: &str) -> Result<bool> {
  use std::io::{Read, Write};
  const DETACH: u8 = 0x1d;
  let mut c = client(cli).await?;
  let (keys, input) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
  // Raw while attached, and back however it ends (an error, a panic).
  struct Raw;
  impl Drop for Raw {
    fn drop(&mut self) {
      let _ = crossterm::terminal::disable_raw_mode();
    }
  }
  crossterm::terminal::enable_raw_mode()?;
  let raw = Raw;
  // Stdin is read on a thread of its own: it blocks.
  std::thread::spawn(move || {
    let mut stdin = std::io::stdin();
    let mut buf = [0u8; 4096];
    while let Ok(n) = stdin.read(&mut buf) {
      if n == 0 {
        break;
      }
      let bytes = &buf[..n];
      match bytes.iter().position(|b| *b == DETACH) {
        Some(i) => {
          if i > 0 {
            let _ = keys.send(bytes[..i].to_vec());
          }
          break;
        }
        None => {
          if keys.send(bytes.to_vec()).is_err() {
            break;
          }
        }
      }
    }
  });
  let mut out = std::io::stdout();
  let ended = c
    .attach(id, input, |bytes| {
      let _ = out.write_all(bytes);
      let _ = out.flush();
    })
    .await;
  drop(raw);
  Ok(ended?)
}

fn stop_node(cli: &Cli) -> Result<bool> {
  let data = data_dir(cli)?;
  if !stitch_node::running(&data) {
    return Ok(false);
  }
  let pid = stitch_node::node_pid(&data);
  // Ask over the socket; a node of another version cannot be asked, so it
  // gets SIGTERM, and SIGKILL if that does not end it.
  let asked = runtime()?.block_on(async {
    match Client::connect(&data).await {
      Ok(mut c) => c.request(&Request::Stop).await.is_ok(),
      Err(_) => false,
    }
  });
  let wait = |tenths: u32| {
    for _ in 0..tenths {
      if !stitch_node::running(&data) {
        return true;
      }
      std::thread::sleep(Duration::from_millis(100));
    }
    false
  };
  if asked && wait(100) {
    return Ok(true);
  }
  let Some(pid) = pid else {
    return Err("the node did not stop, and its lock names no process".into());
  };
  for (signal, tenths) in [("-TERM", 50), ("-KILL", 30)] {
    let _ = std::process::Command::new("kill")
      .args([signal, &pid.to_string()])
      .status();
    if wait(tenths) {
      return Ok(true);
    }
  }
  Err(format!("the node (pid {pid}) did not stop").into())
}

/// The TUI, on the place it runs in.
fn tui(cli: &Cli) -> Result<()> {
  let data = data_dir(cli)?;
  let mut index = open_index(cli)?;
  let roots = all_roots(&mut index)?;
  if !roots.iter().any(|r| r.config().role == Role::Home) {
    return Err("no home workspace: run `stitch setup --name <name>` first".into());
  }
  let places = place::Places::load(&index);
  let (here, checkout) = here_and_checkout(cli, &roots, &places)?;
  let node = node_name(cli)?;
  if let Some(home) = roots.iter().find(|r| r.config().role == Role::Home) {
    place::fold(&mut index, home, &node, &here, checkout.as_ref())?;
  }
  let places = place::Places::load(&index);
  // A project seen for the first time is not listed yet: its checkout names it.
  let here_label = match (&here, checkout) {
    (Place::Project(repo), Some(co)) if !places.projects.iter().any(|p| &p.repo == repo) => {
      co.name()
    }
    _ => here.label(node.as_str(), &places),
  };
  let store = tui::Store {
    node,
    here,
    here_label,
    blobs: BlobStore::open(&data.join("blobs"))?,
    index: std::sync::Mutex::new(index),
    data,
  };
  // The node resolves synced/project defaults and deliberate thread
  // choices; config.toml alone is not the effective model.
  tui::run(store, "asking node…".into())
}

fn run(cli: Cli) -> Result<()> {
  stitch_node::set_build(env!("STITCH_BUILD"));
  let Some(command) = &cli.command else {
    return tui(&cli);
  };
  match command {
    Command::Setup {
      name,
      home,
      no_home,
      local,
    } => {
      let data = data_dir(&cli)?;
      let name: NodeName = name.parse()?;
      let dir = home
        .clone()
        .unwrap_or_else(|| stitch_node::setup::default_home(&data));
      let set =
        stitch_node::setup::setup(&data, &name, (!no_home).then_some(&*dir), *local, false)?;
      println!("node {name} ({})", set.key);
      if let Some(root) = set.home {
        println!("home workspace {}", root.dir().display());
      }
    }
    Command::Vault { action } => vault::run(&cli, action)?,
    Command::Proxy { dir, action } => {
      let data = data_dir(&cli)?;
      let dir = dir.clone().unwrap_or_else(|| data.join("proxy"));
      match action {
        None | Some(ProxyCommand::Serve { .. }) => {
          let listen = match action {
            Some(ProxyCommand::Serve { listen }) if !listen.is_empty() => listen.clone(),
            _ => {
              let c = stitch_proxy::Config::load(&dir)?;
              if c.listen.is_empty() {
                stitch_proxy::config::DEFAULT_LISTEN
                  .iter()
                  .map(|s| s.to_string())
                  .collect()
              } else {
                c.listen
              }
            }
          };
          // API keys only: `keys::get` never reads a subscription's
          // sign-in, which is not to be shared with others' clients.
          let keys: stitch_proxy::Keys = {
            let data = data.clone();
            std::sync::Arc::new(move |name: &str, kind: &str| {
              stitch_node::keys::get(&data, name, kind)
            })
          };
          let proxy = stitch_proxy::Proxy::new(&dir, keys)?;
          eprintln!("proxy: configuration and usage in {}", dir.display());
          runtime()?.block_on(async {
            tokio::select! {
              r = stitch_proxy::run(proxy, &listen) => r?,
              _ = tokio::signal::ctrl_c() => {}
            }
            Ok::<_, Box<dyn Error>>(())
          })?;
        }
        Some(ProxyCommand::Token(ProxyTokenCommand::Add {
          name,
          daily_tokens,
          daily_tokens_per_user,
          rpm,
          upstreams,
        })) => {
          let clear = stitch_proxy::add_token(
            &dir,
            name,
            stitch_proxy::config::Token {
              upstreams: upstreams.clone(),
              daily_tokens: *daily_tokens,
              daily_tokens_per_user: *daily_tokens_per_user,
              requests_per_minute: *rpm,
              ..Default::default()
            },
          )?;
          println!("{clear}");
          eprintln!(
            "[token `{name}`: shown this once, only its hash is kept in {}; a client uses it as its API key]",
            stitch_proxy::Config::path(&dir).display()
          );
        }
        Some(ProxyCommand::Token(ProxyTokenCommand::Rm { name })) => {
          stitch_proxy::remove_token(&dir, name)?;
        }
        Some(ProxyCommand::Token(ProxyTokenCommand::Ls)) => {
          for l in stitch_proxy::report(&dir, 0)? {
            println!("{l}");
          }
        }
        Some(ProxyCommand::Usage { days }) => {
          for l in stitch_proxy::report(&dir, *days)? {
            println!("{l}");
          }
        }
      }
    }
    Command::Discord {
      guilds,
      turns_per_hour,
      model,
      announce,
      servers,
      approvers,
    } => {
      let o = stitch_discord::Options {
        token: stitch_discord::token_from_env()?,
        guilds: stitch_discord::parse_guilds(guilds)?,
        turns_per_hour: *turns_per_hour,
        model: model.clone(),
        data: data_dir(&cli)?,
        announce: *announce,
        servers: servers.clone().filter(|d| !d.as_os_str().is_empty()),
        approvers: stitch_discord::parse_guilds(approvers)
          .map_err(|e| e.replace("guild", "user"))?,
      };
      runtime()?.block_on(stitch_discord::run(o))?;
    }
    Command::Node {
      action: None,
      after,
    } => {
      let data = data_dir(&cli)?;
      noderun::run(&data, *after)?;
    }
    Command::Node {
      action: Some(NodeCommand::Stop { when_idle: false }),
      ..
    } => {
      if !stop_node(&cli)? {
        println!("no node runs");
      }
    }
    Command::Node {
      action: Some(NodeCommand::Stop { when_idle: true }),
      ..
    }
    | Command::Node {
      action: Some(NodeCommand::Restart { .. }),
      ..
    } => {
      let (now, successor) = match command {
        Command::Node {
          action: Some(NodeCommand::Restart { now }),
          ..
        } => (*now, true),
        _ => (false, false),
      };
      let data = data_dir(&cli)?;
      runtime()?.block_on(async {
        match Client::connect(&data).await {
          Ok(mut c) => match c.request(&Request::Restart { now, successor }).await? {
            Reply::Lines(lines) => lines.iter().for_each(|l| println!("{l}")),
            Reply::Error(e) => return Err(e.into()),
            other => return Err(format!("unexpected reply: {other:?}").into()),
          },
          Err(stitch_node::Error::NotRunning) => println!("no node runs"),
          Err(e) => return Err(e.into()),
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Node {
      action: Some(NodeCommand::Service { action }),
      ..
    } => {
      let data = data_dir(&cli)?;
      let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
      let said = match action {
        ServiceCommand::Install {
          update,
          follow,
          promote,
          testflight,
          app,
          exe,
          print,
        } => {
          let exe = match exe {
            Some(e) => e.clone(),
            None => std::env::current_exe()?,
          };
          let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
          if exe.components().any(|c| c.as_os_str() == "target") {
            eprintln!(
              "note: {} is a build output; the service would run it (install with `just install` and run this from the installed stitch, or pass --exe)",
              exe.display()
            );
          }
          let expand = |c: &String| match c.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(c),
          };
          let update = update.as_ref().map(expand);
          // `--app <checkout>` or `--testflight <app checkout>`:
          // STITCH_APP; a plain `--testflight` alone, none (the self-update
          // uses the checkout's own justfile when it holds the app).
          let named = testflight.as_ref().filter(|a| !a.is_empty());
          if let (Some(a), Some(t)) = (app, named)
            && expand(a) != expand(t)
          {
            return Err(format!("--app {a} and --testflight {t}: one app checkout").into());
          }
          let app = app
            .as_ref()
            .or(named)
            .map(expand)
            .map(|a| std::fs::canonicalize(&a).unwrap_or(a));
          let spec = service::Spec {
            exe,
            data: data.clone(),
            home: home.clone(),
            path: std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into()),
            update,
            follow: follow.clone(),
            promote: promote.clone(),
            testflight: testflight.is_some(),
            app,
          };
          service::install(&spec, *print)?
        }
        ServiceCommand::Uninstall => service::uninstall(&data, &home)?,
      };
      said.iter().for_each(|l| println!("{l}"));
    }
    Command::Node {
      action: Some(NodeCommand::Builds { healthy_for }),
      ..
    } => {
      let data = data_dir(&cli)?;
      let (lines, ok) = noderun::builds_said(&data, *healthy_for)?;
      lines.iter().for_each(|l| println!("{l}"));
      if !ok {
        return Err("not healthy for that long".into());
      }
    }
    Command::Node {
      action: Some(NodeCommand::Status),
      ..
    } => {
      let data = data_dir(&cli)?;
      runtime()?.block_on(async {
        match Client::connect(&data).await {
          Ok(mut c) => {
            if let Reply::Status {
              node,
              endpoint,
              repos,
              peers,
              build,
              pending,
            } = c.request(&Request::Status).await?
            {
              println!("node {node} ({endpoint})");
              println!("build: {build}");
              if build != stitch_node::build() {
                println!("  (this stitch is {})", stitch_node::build());
              }
              if let Some(p) = pending {
                println!("{p}");
              }
              println!("repositories: {}", repos.join(", "));
              println!("peers: {}", peers.join(", "));
            }
          }
          Err(stitch_node::Error::NotRunning) => println!("no node runs"),
          Err(e) => println!("{e}"),
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Chat {
      text,
      thread,
      global,
      attachments,
      model,
      repo,
      quiet,
      detach,
      parent,
      budget_usd,
      budget_tokens,
      minutes,
      max_steps,
      on,
      context,
      no_tools,
      no_wake,
      memory,
      read_only,
      tools,
      skills,
      scopes,
    } => {
      let scopes = scopes
        .iter()
        .map(|t| stitch_node::scopes::parse_target(t))
        .collect::<std::result::Result<Vec<_>, _>>()?;
      let text = match text.as_deref() {
        None | Some("-") => read_stdin()?,
        Some(t) => t.to_owned(),
      };
      if text.trim().is_empty() {
        return Err("nothing to send".into());
      }
      let attachments = attachments
        .iter()
        .map(|p| std::fs::canonicalize(p).map(|p| p.display().to_string()))
        .collect::<std::io::Result<Vec<_>>>()?;
      if *detach {
        if !attachments.is_empty() || thread.is_some() {
          return Err("a detached thread starts from its goal: no --thread or --attach".into());
        }
        let req = Request::Detach {
          goal: text,
          model: model.clone(),
          repo: repo.clone(),
          cwd: std::env::current_dir()
            .ok()
            .map(|d| d.display().to_string()),
          parent: parent.clone(),
          budget_usd: *budget_usd,
          budget_tokens: *budget_tokens,
          budget_minutes: *minutes,
          max_steps: *max_steps,
          global: *global,
          node: on.clone(),
          context: context.clone(),
          no_tools: *no_tools,
          tools: tools.clone(),
          skills: skills.clone(),
          keep: *no_wake,
          memory: *memory,
          read_only: *read_only,
          scopes: scopes.clone(),
        };
        runtime()?.block_on(async {
          let mut c = client(&cli).await?;
          match c.request(&req).await? {
            Reply::Detached(id) => {
              println!("{id}");
              eprintln!("[detached; `stitch threads` shows it, `stitch cancel {id}` stops it]");
            }
            Reply::Lines(lines) => {
              for l in lines {
                println!("{l}");
              }
            }
            _ => {}
          }
          Ok::<_, Box<dyn Error>>(())
        })?;
        return Ok(());
      }
      let req = Request::Chat {
        thread: thread.clone(),
        text,
        attachments,
        model: model.clone(),
        repo: repo.clone(),
        cwd: std::env::current_dir()
          .ok()
          .map(|d| d.display().to_string()),
        global: *global,
        env: stitch_node::client::client_env(),
        tools: tools.clone(),
        skills: skills.clone(),
        place: None,
        memory: None,
        end_user: None,
        node: None,
        spoken: false,
        scopes,
      };
      let dim = std::io::stderr().is_terminal();
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let mut thinking = false;
        let done = c
          .chat(&req, |r| {
            let paint = |t: &str| if dim { format!("\x1b[2m{t}\x1b[0m") } else { t.to_owned() };
            match r {
              Reply::Text(t) => {
                if thinking {
                  eprintln!();
                  thinking = false;
                }
                print!("{t}");
                let _ = std::io::stdout().flush();
              }
              Reply::Thinking(t) if !quiet => {
                thinking = true;
                eprint!("{}", paint(t));
              }
              Reply::ToolCall(line) => {
                if thinking {
                  eprintln!();
                  thinking = false;
                }
                eprintln!("{}", paint(line));
              }
              Reply::ToolResult { ok, output } => {
                let lines: Vec<&str> = output.lines().collect();
                let mut shown = lines.iter().take(8).copied().collect::<Vec<_>>().join("\n");
                if lines.len() > 8 {
                  shown.push_str(&format!("\n… {} more lines", lines.len() - 8));
                }
                let mark = if *ok { "" } else { "✗ " };
                eprintln!("{}", paint(&format!("{mark}{shown}")));
              }
              Reply::App {
                name,
                url: Some(url),
              } => {
                eprintln!("App \"{name}\": {url}");
              }
              Reply::Approve {
                command,
                session,
                grant,
                ..
              } => {
                // No terminal to ask: it waits on the attention list.
                let decision = ask_approval(session, command, grant)?;
                let (answer, decided) = tokio::sync::oneshot::channel();
                let _ = answer.send(decision);
                return Some(decided);
              }
              Reply::Note(t) => eprintln!("\n{}", paint(t)),
              Reply::Compacted(c) => {
                eprintln!("\n{}", paint(&format!("── {} (thread {}) ──", c.said(), c.to)))
              }
              Reply::Fetched(n) => eprintln!("[fetched {n} attachment(s) from peers]"),
              Reply::Kept => eprintln!(
                "\n{}",
                paint("[the model call failed: your message is kept in the thread; send another to go on]")
              ),
              Reply::Retry(why) => {
                eprintln!("\n{}", paint(&format!("[cut off by the network, sending it again: {why}]")))
              }
              Reply::Waiting(true) => {
                eprintln!("{}", paint("[a turn already runs on this thread: this waits for it to end]"))
              }
              Reply::Afterthought(t) => {
                println!("\n");
                eprintln!("{}", paint("[memory: an afterthought]"));
                print!("{t}");
                let _ = std::io::stdout().flush();
              }
              _ => {}
            }
            None
          })
          .await?;
        println!();
        if let Reply::Unsent = done {
          eprintln!("[interrupted before it was sent: the message was not sent]");
        }
        if let Reply::ChatDone {
          thread,
          model,
          stop,
          input_tokens,
          output_tokens,
          cache_read_tokens,
          cache_write_tokens,
          usd,
          forked,
          ..
        } = done
        {
          let tokens = stitch_node::api::tokens_text(
            input_tokens,
            cache_read_tokens,
            cache_write_tokens,
            output_tokens,
          );
          let cost = usd.map(|u| format!(" · ${u:.4}")).unwrap_or_default();
          let stop = if stop == "end_turn" { String::new() } else { format!(" · {stop}") };
          eprintln!(
            "[{thread} · {model} · {tokens}{cost}{stop}]"
          );
          if forked {
            eprintln!("[this thread was continued on two nodes; the longest is followed, and the model is told what the other did]");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Threads { all } => {
      let mut index = open_index(&cli)?;
      let roots = all_roots(&mut index)?;
      let here = here_folded(&cli, &mut index, &roots)?;
      let places = place::Places::load(&index);
      let home = roots
        .iter()
        .find(|r| r.config().role == Role::Home)
        .map(|r| r.repo().clone());
      let node = node_name(&cli)?;
      for (r, at, p) in place::listed(index.listed_threads()?, home.as_ref(), &here, *all, &places)
      {
        let when = at
          .to_zoned(jiff::tz::TimeZone::system())
          .strftime("%Y-%m-%d %H:%M");
        let state = stitch_node::chat::detached_state(&index, &r)?
          .map(|s| format!("[{s}] "))
          .unwrap_or_default();
        let place = if p == here {
          String::new()
        } else {
          format!("  · {}", p.label(node.as_str(), &places))
        };
        println!(
          "{}  {when}  {state}{}{place}",
          r.id,
          index.lineage_title(&r.id).unwrap_or_default()
        );
      }
      // Spawned on another node and not synced back yet: there already,
      // and reachable (`stitch send`).
      for a in index.records(Some(stitch_records::kinds::ASSIGNMENT))? {
        let field = |k: &str| a.fields.get(k).and_then(|v| v.as_str()).map(str::to_owned);
        let (Some(id), Some(on)) = (field("thread"), field("node")) else {
          continue;
        };
        let continues = a.fields.get("continue").and_then(|v| v.as_bool()) == Some(true);
        let Ok(id) = id.parse::<RecordId>() else {
          continue;
        };
        if continues || index.record(&id)?.is_some() {
          continue;
        }
        let when = a
          .created
          .to_zoned(jiff::tz::TimeZone::system())
          .strftime("%Y-%m-%d %H:%M");
        println!(
          "{id}  {when}  [starting on {on}] {}",
          index.title(&a.id)?.unwrap_or_default()
        );
      }
    }
    Command::Answer {
      question,
      decision,
      reason,
    } => {
      let (allow, always) = match decision.as_str() {
        "y" | "yes" | "allow" => (true, false),
        "a" | "always" => (true, true),
        "n" | "no" | "decline" => (false, false),
        other => return Err(format!("`{other}`: answer y, n or always").into()),
      };
      let reason = (!reason.is_empty()).then(|| reason.join(" "));
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        c.request(&Request::Answer {
          question: question.clone(),
          allow,
          reason,
          always,
        })
        .await?;
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Send {
      thread,
      text,
      from,
      no_wake,
      interject,
      scopes,
    } => {
      let scopes = scopes
        .iter()
        .map(|t| stitch_node::scopes::parse_target(t))
        .collect::<std::result::Result<Vec<_>, _>>()?;
      let text = match text.as_deref() {
        None | Some("-") => read_stdin()?,
        Some(t) => t.to_owned(),
      };
      let req = Request::Send {
        thread: thread.clone(),
        text,
        from: from.clone(),
        keep: *no_wake,
        interject: *interject,
        scopes,
      };
      runtime()?.block_on(async {
        if let Reply::Lines(lines) = client(&cli).await?.request(&req).await? {
          for l in lines {
            println!("{l}");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Notify {
      body,
      title,
      thread,
    } => {
      let body = match body.as_deref() {
        None | Some("-") => read_stdin()?,
        Some(t) => t.to_owned(),
      };
      let req = Request::Notify {
        title: title.clone(),
        body,
        thread: thread.clone(),
      };
      runtime()?.block_on(async {
        if let Reply::Lines(lines) = client(&cli).await?.request(&req).await? {
          for l in lines {
            println!("{l}");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Issue { action, thread } => {
      // A bundle made here for `push`, removed once the node read it.
      let mut bundle: Option<PathBuf> = None;
      // Where it runs: the repository a fixer's command means, when it
      // names none (a hint: the node checks what it pushes and lands).
      let cwd = std::env::current_dir()
        .ok()
        .map(|d| d.display().to_string());
      let input = match action {
        IssueCommand::Push {
          issue,
          branches,
          repo,
        } => {
          let branches = if branches.is_empty() {
            let out = std::process::Command::new("git")
              .args(["symbolic-ref", "--short", "HEAD"])
              .output()?;
            if !out.status.success() {
              return Err("no branch checked out here: name the branches to push".into());
            }
            vec![String::from_utf8_lossy(&out.stdout).trim().to_owned()]
          } else {
            branches.clone()
          };
          let path = std::env::temp_dir().join(format!(
            "stitch-push-{}-{}.bundle",
            std::process::id(),
            std::time::SystemTime::now()
              .duration_since(std::time::UNIX_EPOCH)
              .map_or(0, |d| d.as_nanos())
          ));
          let out = std::process::Command::new("git")
            .args(["bundle", "create", "--quiet"])
            .arg(&path)
            .args(branches.iter().map(|b| format!("refs/heads/{b}")))
            .output()?;
          if !out.status.success() {
            let _ = std::fs::remove_file(&path);
            return Err(
              format!(
                "git bundle create: {}",
                String::from_utf8_lossy(&out.stderr).trim()
              )
              .into(),
            );
          }
          bundle = Some(path.clone());
          serde_json::json!({"action": "push", "issue": issue, "bundle": path, "repo": repo, "cwd": cwd})
        }
        IssueCommand::Ci {
          issue,
          commit,
          repo,
        } => {
          serde_json::json!({"action": "ci", "issue": issue, "commit": commit, "repo": repo, "cwd": cwd})
        }
        IssueCommand::Fetch { repo } => serde_json::json!({"action": "fetch", "repo": repo}),
        IssueCommand::Revert { issue } => {
          serde_json::json!({"action": "revert", "issue": issue})
        }
        IssueCommand::List { all } => serde_json::json!({"action": "list", "all": all}),
        IssueCommand::Show { issue } => serde_json::json!({"action": "show", "issue": issue}),
        IssueCommand::Land {
          issue,
          commit,
          summary,
          repo,
        } => serde_json::json!({
          "action": "land", "issue": issue, "commit": commit, "summary": summary,
          "repo": repo, "cwd": cwd,
        }),
        IssueCommand::Report {
          title,
          what,
          expected,
          evidence,
          area,
        } => serde_json::json!({
          "action": "report", "title": title, "what_happened": what,
          "expected": expected, "evidence": evidence, "area": area,
        }),
        IssueCommand::Set {
          issue,
          state,
          note,
          fix,
          repo,
        } => serde_json::json!({
          "action": "set", "issue": issue, "state": state, "note": note, "fix": fix,
          "repo": repo,
        }),
        IssueCommand::Fix {
          issue,
          rollout,
          budget_usd,
          model,
          repo,
        } => serde_json::json!({
          "action": "fix", "issue": issue, "rollout": rollout,
          "budget_usd": budget_usd, "model": model, "repo": repo,
        }),
        IssueCommand::Schedule => serde_json::json!({"action": "schedule"}),
        IssueCommand::Settings {
          node,
          place,
          budget,
          daily,
          every,
          model,
          on,
          off,
          mode,
          auto_land,
          places,
        } => serde_json::json!({
          "action": "settings", "node": node, "place": place, "budget_usd": budget,
          "daily_usd": daily, "every": every, "model": model,
          "on": if *on { Some(true) } else if *off { Some(false) } else { None },
          "mode": mode, "auto_land": auto_land.as_deref().map(|a| a == "on"),
          "places": places,
        }),
      };
      let req = Request::Issue {
        input: input.to_string(),
        thread: thread.clone(),
      };
      let done = runtime()?.block_on(async {
        if let Reply::Lines(lines) = client(&cli).await?.request(&req).await? {
          for l in lines {
            println!("{l}");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      });
      if let Some(b) = bundle {
        let _ = std::fs::remove_file(b);
      }
      done?;
    }
    Command::Mcp => runtime()?.block_on(check_mcp(&data_dir(&cli)?))?,
    Command::Decide { command } => {
      let input = match command.clone().unwrap_or(DecideCommand::Profiles) {
        DecideCommand::Profiles => serde_json::json!({"action": "profiles"}),
        DecideCommand::Set {
          profile,
          thresholds,
          fallback,
          model,
        } => serde_json::json!({
          "action": "set", "profile": profile, "thresholds": thresholds,
          "fallback": fallback, "model": model,
        }),
        DecideCommand::Try {
          profile,
          input,
          yes_no,
        } => {
          let input = if input == "-" {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
            s
          } else {
            input
          };
          let questions: serde_json::Map<String, serde_json::Value> = yes_no
            .iter()
            .enumerate()
            .map(|(i, q)| (format!("q{}", i + 1), serde_json::json!(q)))
            .collect();
          serde_json::json!({
            "action": "try", "profile": profile, "input": input,
            "questions": if questions.is_empty() { serde_json::Value::Null } else { serde_json::Value::Object(questions) },
          })
        }
        DecideCommand::Log { profile, n } => {
          serde_json::json!({"action": "log", "profile": profile, "n": n})
        }
      };
      let req = Request::Decide {
        input: input.to_string(),
      };
      runtime()?.block_on(async {
        match client(&cli).await?.request(&req).await? {
          Reply::Lines(lines) => {
            for l in lines {
              println!("{l}");
            }
            Ok::<_, Box<dyn Error>>(())
          }
          Reply::Error(e) => Err(e.into()),
          other => Err(format!("unexpected reply: {other:?}").into()),
        }
      })?;
    }
    Command::Yolo { state } => {
      let on = state.as_deref().map(|s| s == "on");
      runtime()?.block_on(async {
        match client(&cli).await?.request(&Request::Yolo { on }).await? {
          Reply::Lines(lines) => lines.iter().for_each(|l| println!("{l}")),
          Reply::Error(e) => return Err(e.into()),
          _ => {}
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Mode { mode } => {
      runtime()?.block_on(async {
        match client(&cli)
          .await?
          .request(&Request::Mode {
            mode: mode.clone(),
            cwd: std::env::current_dir()
              .ok()
              .map(|d| d.display().to_string()),
          })
          .await?
        {
          Reply::Lines(lines) => lines.iter().for_each(|l| println!("{l}")),
          Reply::Error(e) => return Err(e.into()),
          _ => {}
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Shell {
      action: ShellCommand::Attach { id },
    } => {
      let ended = runtime()?.block_on(attach(&cli, id))?;
      println!(
        "\r\n{}",
        if ended {
          "the session ended"
        } else {
          "detached"
        }
      );
    }
    Command::Jobs { action, id } => {
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        match (action.as_deref(), id) {
          (Some("stop"), Some(id)) => {
            if let Reply::Lines(lines) = c.request(&Request::StopJob { id: id.clone() }).await? {
              for l in lines {
                println!("{l}");
              }
            }
          }
          (Some(_), None) => return Err("which job? `stitch jobs` lists their IDs".into()),
          _ => {
            let Reply::Jobs(jobs) = c.request(&Request::Jobs).await? else {
              return Err("unexpected reply".into());
            };
            if jobs.is_empty() {
              println!("nothing runs in the background");
            }
            for j in jobs {
              let ago = j
                .since
                .parse::<jiff::Timestamp>()
                .ok()
                .map(|t| {
                  stitch_node::tools::spoken(
                    jiff::Timestamp::now().duration_since(t).unsigned_abs(),
                  )
                })
                .unwrap_or_default();
              let title = if j.title.is_empty() {
                &j.thread
              } else {
                &j.title
              };
              println!("{} {}", j.kind, j.id);
              println!("  {}", j.what);
              println!("  {} · for {ago} · on {} · {title}", j.state, j.node);
            }
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Rename { thread, title } => {
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        c.request(&Request::Rename {
          thread: thread.clone(),
          title: title.join(" "),
        })
        .await?;
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Instructions { global, edit, set } => {
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let cwd = std::env::current_dir()
          .ok()
          .map(|d| d.display().to_string());
        let ask = |set: Option<String>| Request::Instructions {
          place: None,
          cwd: cwd.clone(),
          global: *global,
          set,
        };
        let Reply::Instructions { place, text } = c.request(&ask(None)).await? else {
          return Err("unexpected reply".into());
        };
        let new = match (set, edit) {
          (Some(p), _) if p.as_os_str() == "-" => {
            let mut t = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut t)?;
            Some(t)
          }
          (Some(p), _) => Some(std::fs::read_to_string(p)?),
          (None, true) => edited(&place, text.as_deref().unwrap_or(""))?,
          (None, false) => {
            match &text {
              Some(t) => println!("# Yours, for {place}\n\n{}", t.trim_end()),
              None => println!("No instructions of yours for {place}."),
            }
            return Ok(());
          }
        };
        let Some(new) = new.filter(|n| Some(n.as_str()) != text.as_deref()) else {
          println!("Unchanged: {place}.");
          return Ok(());
        };
        let Reply::Instructions { place, text } = c.request(&ask(Some(new))).await? else {
          return Err("unexpected reply".into());
        };
        match text {
          Some(_) => println!("Instructions set for {place}."),
          None => println!("Instructions cleared for {place}."),
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Pin { thread, off } => {
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        c.request(&Request::Pin {
          thread: thread.clone(),
          on: !*off,
        })
        .await?;
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Artefacts { thread } => {
      let v = runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let Reply::Lines(l) = c
          .request(&Request::Artefacts {
            thread: thread.clone(),
          })
          .await?
        else {
          return Err("unexpected reply".into());
        };
        Ok::<_, Box<dyn Error>>(serde_json::from_str::<serde_json::Value>(&l[0])?)
      })?;
      for kind in ["path", "url", "commit", "thread"] {
        let items = v["kinds"][kind].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
          continue;
        }
        println!("{kind}:");
        for i in items {
          let g = |k: &str| i[k].as_str().unwrap_or("");
          println!(
            "  {:7} {:>3}x  {}  {}{}  ({})",
            g("verb"),
            i["n"],
            g("last_at"),
            g("value"),
            i["subject"]
              .as_str()
              .map(|s| format!("  {s}"))
              .unwrap_or_default(),
            g("msg"),
          );
        }
      }
    }
    Command::Room { action } => {
      let req = match action {
        RoomCommand::New {
          participants,
          title,
          place,
          scopes,
          opts,
        } => Request::RoomCreate {
          title: title.clone(),
          place: place.clone(),
          from_thread: None,
          participants: participants
            .iter()
            .map(|p| room_participant(p, opts.writable, opts.no_tools))
            .collect(),
          scopes: scopes
            .iter()
            .map(|t| stitch_node::scopes::parse_target(t))
            .collect::<std::result::Result<Vec<_>, _>>()?,
          mode: opts.mode.clone(),
          hop_cap: opts.hop_cap,
          budget_usd: opts.budget_usd,
        },
        RoomCommand::Pull {
          thread,
          models,
          opts,
        } => Request::RoomCreate {
          title: None,
          place: None,
          from_thread: Some(thread.clone()),
          participants: models
            .iter()
            .map(|p| room_participant(p, opts.writable, opts.no_tools))
            .collect(),
          scopes: Vec::new(),
          mode: opts.mode.clone(),
          hop_cap: opts.hop_cap,
          budget_usd: opts.budget_usd,
        },
        RoomCommand::Show { room } => Request::RoomInfo {
          thread: room.clone(),
        },
        RoomCommand::Set {
          room,
          add,
          remove,
          mute,
          unmute,
          mode,
          hop_cap,
          writable,
          no_tools,
        } => Request::RoomUpdate {
          thread: room.clone(),
          add: add
            .iter()
            .map(|p| room_participant(p, *writable, *no_tools))
            .collect(),
          remove: remove.clone(),
          mute: mute.clone(),
          unmute: unmute.clone(),
          mode: mode.clone(),
          hop_cap: *hop_cap,
        },
      };
      runtime()?.block_on(async {
        if let Reply::Lines(lines) = client(&cli).await?.request(&req).await? {
          for l in lines {
            print_room(&l);
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Scope { targets, search } => {
      let scopes = targets
        .iter()
        .map(|t| stitch_node::scopes::parse_target(t))
        .collect::<std::result::Result<Vec<_>, _>>()?;
      let req = Request::Scope {
        scopes,
        query: search.clone(),
      };
      runtime()?.block_on(async {
        if let Reply::Lines(lines) = client(&cli).await?.request(&req).await? {
          for l in lines {
            println!("{l}");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Digest {
      thread: None,
      sweep,
      ..
    }
    | Command::Digest {
      thread: Some(_),
      status: true,
      sweep,
      ..
    } => {
      let v = runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let Reply::Lines(l) = c
          .request(&Request::DigestStatus {
            sweep: Some(*sweep),
          })
          .await?
        else {
          return Err("unexpected reply".into());
        };
        Ok::<_, Box<dyn Error>>(serde_json::from_str::<serde_json::Value>(&l[0])?)
      })?;
      let n = |k: &str| v[k].as_u64().unwrap_or(0);
      println!(
        "{} conversations: {} with a fresh digest, {} stale, {} none; this node's sweep has {} due",
        n("conversations"),
        n("fresh"),
        n("stale"),
        n("none"),
        n("due_here"),
      );
      println!(
        "today ${:.4} of ${:.2}; digests {}, sweep {}{}",
        v["day_usd"].as_f64().unwrap_or(0.0),
        v["cap_usd"].as_f64().unwrap_or(0.0),
        if v["enabled"].as_bool() == Some(true) {
          "on"
        } else {
          "off"
        },
        if v["backfill"].as_bool() == Some(true) {
          "on"
        } else {
          "off"
        },
        if v["running"].as_bool() == Some(true) {
          " (running now)"
        } else {
          ""
        },
      );
      for (label, w) in [
        ("last sweep", &v["last_sweep"]),
        ("this sweep", &v["sweep"]),
      ] {
        if w.is_object() {
          println!(
            "{label}: {}: {} of {} due digested, {} set aside, ${:.4}{}{}",
            w["at"].as_str().unwrap_or(""),
            w["digested"].as_array().map_or(0, Vec::len),
            w["due"],
            w["skipped"],
            w["usd"].as_f64().unwrap_or(0.0),
            if w["capped"].as_bool() == Some(true) {
              ", the day's cap reached"
            } else {
              ""
            },
            w["failed"]
              .as_array()
              .filter(|f| !f.is_empty())
              .map(|f| format!(", {} failed", f.len()))
              .unwrap_or_default(),
          );
        }
      }
    }
    Command::Digest {
      thread: Some(thread),
      update,
      ..
    } => {
      let v = runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let Reply::Lines(l) = c
          .request(&Request::Digest {
            thread: thread.clone(),
            update: Some(*update),
          })
          .await?
        else {
          return Err("unexpected reply".into());
        };
        Ok::<_, Box<dyn Error>>(serde_json::from_str::<serde_json::Value>(&l[0])?)
      })?;
      if let Some(u) = v.get("update") {
        let g = |k: &str| u[k].as_str().unwrap_or("");
        println!(
          "update: {}{} ({} windows{}, {} items dropped), ${:.4} on {}; today ${:.4} of ${:.2}",
          g("outcome"),
          u["why"]
            .as_str()
            .map(|w| format!(": {w}"))
            .unwrap_or_default(),
          u["windows"],
          if u["more"].as_bool() == Some(true) {
            ", more to go"
          } else {
            ""
          },
          u["dropped"],
          u["usd"].as_f64().unwrap_or(0.0),
          g("model"),
          u["day_usd"].as_f64().unwrap_or(0.0),
          u["cap_usd"].as_f64().unwrap_or(0.0),
        );
      }
      let d = &v["digest"];
      if d.is_null() {
        println!("No digest for {}.", v["thread"].as_str().unwrap_or(thread));
      } else {
        let g = |k: &str| d[k].as_str().unwrap_or("");
        println!(
          "{}\n{}, {} up to {}, ${:.4}, {}{}",
          g("headline"),
          g("at"),
          g("model"),
          g("upto"),
          d["usd"].as_f64().unwrap_or(0.0),
          if d["fresh"].as_bool() == Some(true) {
            "fresh"
          } else {
            "STALE: the thread moved since"
          },
          if g("thread") != v["thread"].as_str().unwrap_or("") {
            format!(" (of the earlier thread {})", g("thread"))
          } else {
            String::new()
          },
        );
        for i in d["items"].as_array().cloned().unwrap_or_default() {
          println!(
            "  {:8} {}  ({})",
            i["kind"].as_str().unwrap_or(""),
            i["text"].as_str().unwrap_or(""),
            i["at"].as_str().unwrap_or(""),
          );
        }
      }
    }
    Command::Touched { value, kind } => {
      let kind = kind.clone().unwrap_or_else(|| {
        let hex = (7..=40).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_hexdigit());
        if value.starts_with("http://") || value.starts_with("https://") {
          "url"
        } else if value.parse::<stitch_records::RecordId>().is_ok() {
          "thread"
        } else if hex {
          "commit"
        } else {
          "path"
        }
        .to_owned()
      });
      let v = runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let Reply::Lines(l) = c
          .request(&Request::Touched {
            kind,
            value: value.clone(),
          })
          .await?
        else {
          return Err("unexpected reply".into());
        };
        Ok::<_, Box<dyn Error>>(serde_json::from_str::<serde_json::Value>(&l[0])?)
      })?;
      let rows = v["conversations"].as_array().cloned().unwrap_or_default();
      if rows.is_empty() {
        println!("Nothing touched {}.", v["value"].as_str().unwrap_or(value));
      }
      for r in rows {
        let g = |k: &str| r[k].as_str().unwrap_or("");
        println!(
          "{}  {:7} {:>3}x  {}  {}  [{}]",
          g("first_at"),
          g("verb"),
          r["n"],
          g("thread"),
          g("title"),
          g("place"),
        );
      }
    }
    Command::Search { query, here, limit } => {
      let hits = runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        match c
          .request(&Request::Search {
            query: query.join(" "),
            here: *here,
            cwd: std::env::current_dir()
              .ok()
              .map(|d| d.display().to_string()),
            limit: *limit,
          })
          .await?
        {
          Reply::Hits(h) => Ok::<_, Box<dyn Error>>(h),
          other => Err(format!("unexpected reply: {other:?}").into()),
        }
      })?;
      if hits.is_empty() {
        eprintln!("nothing found");
      }
      for h in hits {
        println!(
          "{}  {}  {}  [{} {}, by {}]",
          h.thread,
          h.at.get(..10).unwrap_or(&h.at),
          h.title,
          h.msg,
          h.sender,
          h.found_by
        );
        println!("    {}", h.snippet);
      }
    }
    Command::Fork {
      thread,
      upto,
      last,
      no_attachments,
      no_tools,
      summarize,
    } => {
      let thread = if thread == "last" {
        let mut index = open_index(&cli)?;
        let roots = all_roots(&mut index)?;
        let here = here_folded(&cli, &mut index, &roots)?;
        let places = place::Places::load(&index);
        let home = roots
          .iter()
          .find(|r| r.config().role == Role::Home)
          .map(|r| r.repo().clone());
        place::ordered(
          index.listed_threads()?,
          home.as_ref(),
          &here,
          false,
          &places,
        )
        .first()
        .map(|(r, ..)| r.id.to_string())
        .ok_or("no thread yet")?
      } else {
        thread.clone()
      };
      let id = runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        match c
          .request(&Request::Fork {
            thread,
            upto: upto.clone(),
            last: *last,
            no_attachments: *no_attachments,
            no_tools: *no_tools,
            summarize: *summarize,
          })
          .await?
        {
          Reply::Forked(id) => Ok::<_, Box<dyn Error>>(id),
          other => Err(format!("unexpected reply: {other:?}").into()),
        }
      })?;
      println!("{id}");
    }
    Command::Event { action } => event_command(&cli, action)?,
    Command::Schedule { action } => {
      let input = match action {
        ScheduleCommand::Add {
          goal,
          name,
          at,
          every,
          tz,
          place,
          on,
          model,
          max_steps,
          read_only,
          budget_usd,
          budget_tokens,
          budget_minutes,
          daily_cap_usd,
          check,
          check_file,
          fresh,
          no_catch_up,
          reports_to,
          wake,
          on_event,
          debounce,
          max_per_hour,
          until,
          profile,
        } => {
          let mut i = serde_json::json!({
            "on": on_event, "debounce_secs": debounce, "max_per_hour": max_per_hour,
            "until": until,
            "action": "create",
            "goal": goal.join(" "),
            "name": name, "at": at, "every": every, "tz": tz, "place": place,
            "node": on, "model": model, "max_steps": max_steps,
            "budget_usd": budget_usd, "budget_tokens": budget_tokens,
            "budget_minutes": budget_minutes, "daily_cap_usd": daily_cap_usd,
            "check": check, "check_file": check_file, "reports_to": reports_to,
          });
          if *read_only {
            i["read_only"] = true.into();
          }
          if *fresh {
            i["thread"] = "fresh".into();
          }
          if *no_catch_up {
            i["catch_up"] = false.into();
          }
          if *wake {
            i["wake"] = true.into();
          }
          if let Some(p) = profile.value() {
            i["profile"] = p;
          }
          i
        }
        ScheduleCommand::List { all } => serde_json::json!({"action": "list", "all": all}),
        ScheduleCommand::Next => serde_json::json!({"action": "next"}),
        ScheduleCommand::Show { schedule } => {
          serde_json::json!({"action": "show", "schedule": schedule})
        }
        ScheduleCommand::Pause { schedule } => {
          serde_json::json!({"action": "pause", "schedule": schedule})
        }
        ScheduleCommand::Resume { schedule } => {
          serde_json::json!({"action": "resume", "schedule": schedule})
        }
        ScheduleCommand::Run { schedule } => {
          serde_json::json!({"action": "run_now", "schedule": schedule})
        }
        ScheduleCommand::Rm { schedule } => {
          serde_json::json!({"action": "delete", "schedule": schedule})
        }
      };
      // Unset options are not sent.
      let input = match input {
        serde_json::Value::Object(m) => {
          serde_json::Value::Object(m.into_iter().filter(|(_, v)| !v.is_null()).collect())
        }
        v => v,
      };
      let req = Request::Schedule {
        input: input.to_string(),
        cwd: std::env::current_dir()
          .ok()
          .map(|d| d.display().to_string()),
      };
      runtime()?.block_on(async {
        match client(&cli).await?.request(&req).await? {
          Reply::Lines(lines) => {
            for l in lines {
              println!("{l}");
            }
            Ok::<_, Box<dyn Error>>(())
          }
          other => Err(format!("unexpected reply: {other:?}").into()),
        }
      })?;
    }
    Command::Cancel { thread } => {
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        c.request(&Request::Cancel {
          thread: thread.clone(),
        })
        .await?;
        Ok::<_, Box<dyn Error>>(())
      })?;
      println!("cancelled {thread}");
    }
    Command::Thread {
      action: Some(ThreadCommand::Rm {
        ids,
        spawned_by,
        yes,
      }),
      ..
    } => {
      let mut threads = Vec::new();
      for id in ids {
        threads.push(match id.as_str() {
          "last" => last_thread(&cli)?.to_string(),
          _ => id.clone(),
        });
      }
      if threads.is_empty() && spawned_by.is_none() {
        return Err("name the threads to delete (or --spawned-by <id>)".into());
      }
      let req = |dry_run: bool| Request::DeleteThreads {
        threads: threads.clone(),
        spawned_by: spawned_by.clone(),
        dry_run,
        from_thread: std::env::var("STITCH_THREAD")
          .ok()
          .filter(|t| !t.is_empty()),
      };
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let listed = match c.request(&req(true)).await? {
          Reply::Lines(lines) => lines,
          Reply::Error(e) => return Err(e.into()),
          other => return Err(format!("unexpected reply: {other:?}").into()),
        };
        if listed.is_empty() {
          println!("nothing to delete");
          return Ok(());
        }
        let doomed: Vec<doomed::Doomed> = listed.iter().map(|l| doomed::parse(l)).collect();
        for d in &doomed {
          match doomed::label(d) {
            "" => println!("  {}  {}", d.id, d.title),
            label => println!("  {}  {}  ({label})", d.id, d.title),
          }
        }
        if !*yes {
          use std::io::{BufRead, Write};
          print!("{} [y/N] ", doomed::question(&doomed));
          std::io::stdout().flush()?;
          let mut line = String::new();
          std::io::stdin().lock().read_line(&mut line)?;
          if !matches!(line.trim(), "y" | "Y" | "yes") {
            println!("nothing deleted");
            return Ok(());
          }
        }
        match c.request(&req(false)).await? {
          Reply::Lines(lines) => println!("deleted {} thread(s)", lines.len()),
          Reply::Error(e) => return Err(e.into()),
          other => return Err(format!("unexpected reply: {other:?}").into()),
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Thread {
      action:
        Some(ThreadCommand::Share {
          id,
          effort,
          level,
          out,
          yes,
          dry_run,
        }),
      ..
    } => {
      let thread = match id.as_str() {
        "last" => last_thread(&cli)?.to_string(),
        _ => id.clone(),
      };
      let req = |preview: bool, confirm: Option<String>| Request::Share {
        thread: thread.clone(),
        effort: *effort,
        level: level.clone(),
        preview,
        confirm,
      };
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let shared = |reply: Reply| -> Result<stitch_node::share::Preview> {
          match reply {
            Reply::Lines(l) if !l.is_empty() => Ok(serde_json::from_str(&l[0])?),
            Reply::Error(e) => Err(e.into()),
            other => Err(format!("unexpected reply: {other:?}").into()),
          }
        };
        let p = shared(c.request(&req(true, None)).await?)?;
        println!(
          "\"{}\": {} thread(s), {} message(s), {} tool call(s), {} attachment(s) ({} app(s)), \
           level {}, {} KB",
          p.title,
          p.threads,
          p.messages,
          p.calls,
          p.attachments,
          p.apps,
          p.level.as_str(),
          p.bytes.div_ceil(1024)
        );
        if p.redactions > 0 {
          println!("{} secret value(s) redacted", p.redactions);
        }
        for why in &p.left_out {
          println!("not shared: {why}");
        }
        if !p.findings.is_empty() {
          println!("The leak scan found:");
          for f in &p.findings {
            println!(
              "  {:<16} {}  (×{}, first at {})",
              f.kind, f.text, f.count, f.at
            );
          }
        }
        if *dry_run {
          return Ok(());
        }
        if !p.findings.is_empty() && !*yes {
          use std::io::{BufRead, IsTerminal, Write};
          if !std::io::stdin().is_terminal() {
            return Err("not written: review the findings, then run again with --yes".into());
          }
          print!("Share it with these in it? [y/N] ");
          std::io::stdout().flush()?;
          let mut line = String::new();
          std::io::stdin().lock().read_line(&mut line)?;
          if !matches!(line.trim(), "y" | "Y" | "yes") {
            println!("not written");
            return Ok(());
          }
        }
        let written = shared(c.request(&req(false, Some(p.hash.clone()))).await?)?;
        let from = PathBuf::from(written.path.clone().unwrap_or_default());
        let to = out
          .clone()
          .unwrap_or_else(|| PathBuf::from(&written.file_name));
        std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", to.display()))?;
        let _ = std::fs::remove_file(&from);
        if let Some(dir) = from.parent() {
          let _ = std::fs::remove_dir(dir);
        }
        println!("written to {}", to.display());
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Thread {
      action: Some(ThreadCommand::Import { file, yes }),
      ..
    } => {
      let bundle = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let p = import_request(&mut c, &bundle, true).await?;
        let signer = match (&p.signer, p.mine) {
          (Some(_), true) => "signed by this node".to_owned(),
          (Some(k), false) => format!("signed by key {}", &k[..k.len().min(16)]),
          (None, _) => "unsigned".to_owned(),
        };
        println!(
          "\"{}\" ({}, level {}, made {}): {} message(s), {} tool call(s), {} attachment(s) \
           ({} app(s)); {signer}; {}",
          p.title,
          p.kind,
          p.level.as_str(),
          p.created,
          p.messages,
          p.calls,
          p.attachments,
          p.apps,
          p.hash
        );
        for part in &p.parts {
          println!("  {part}");
        }
        if p.already {
          println!("already imported:");
          for t in &p.threads {
            println!("  {t}");
          }
          return Ok(());
        }
        if !*yes {
          use std::io::{BufRead, IsTerminal, Write};
          if !std::io::stdin().is_terminal() {
            return Err("not imported: run again with --yes to import it".into());
          }
          print!("Import it as foreign history (no tools until you lift it)? [y/N] ");
          std::io::stdout().flush()?;
          let mut line = String::new();
          std::io::stdin().lock().read_line(&mut line)?;
          if !matches!(line.trim(), "y" | "Y" | "yes") {
            println!("not imported");
            return Ok(());
          }
        }
        let i = import_request(&mut c, &bundle, false).await?;
        println!("imported:");
        for t in &i.threads {
          println!("  {t}");
        }
        println!(
          "Foreign history: data, not instructions; no tools until `stitch event untaint {}`.",
          i.thread
        );
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Thread {
      action: Some(action),
      ..
    } => {
      let (id, changes, skills) = match action {
        ThreadCommand::Tools { id, changes } => (id, changes, false),
        ThreadCommand::Skills { id, changes } => (id, changes, true),
        ThreadCommand::Rm { .. } | ThreadCommand::Share { .. } | ThreadCommand::Import { .. } => {
          unreachable!("matched above")
        }
      };
      let req = Request::Toolset {
        thread: Some(id.clone()),
        cwd: std::env::current_dir()
          .ok()
          .map(|d| d.display().to_string()),
        global: false,
        tools: if skills { Vec::new() } else { changes.clone() },
        skills: if skills { changes.clone() } else { Vec::new() },
      };
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        match c.request(&req).await? {
          Reply::Toolset {
            thread,
            tools,
            skills: skill_items,
          } => {
            let items = if skills { skill_items } else { tools };
            println!(
              "{} of thread {}:",
              if skills { "skills" } else { "tools" },
              thread.unwrap_or_default()
            );
            for line in set_lines(&items) {
              println!("{line}");
            }
          }
          Reply::Error(e) => return Err(e.into()),
          other => return Err(format!("unexpected reply: {other:?}").into()),
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Thread {
      id,
      action: None,
      read_only,
      writable,
    } => {
      let id = id.clone().ok_or("name a thread (or `last`)")?;
      if *read_only || *writable {
        let id = match id.as_str() {
          "last" => last_thread(&cli)?.to_string(),
          _ => id,
        };
        let reply = runtime()?.block_on(async {
          client(&cli)
            .await?
            .request(&Request::ReadOnly {
              thread: id.clone(),
              on: Some(*read_only),
            })
            .await
            .map_err(Box::<dyn Error>::from)
        })?;
        match reply {
          Reply::Lines(lines) => println!("thread {id}: {}", lines.join(" ")),
          Reply::Error(e) => return Err(e.into()),
          other => return Err(format!("unexpected reply: {other:?}").into()),
        }
        return Ok(());
      }
      let mut index = open_index(&cli)?;
      let roots = all_roots(&mut index)?;
      let id: RecordId = if id == "last" {
        last_thread(&cli)?
      } else {
        id.parse()?
      };
      let repo = index.record(&id)?.ok_or("no such thread here")?.repo;
      let root = roots
        .into_iter()
        .find(|r| r.repo() == &repo)
        .ok_or("its repository is not held here")?;
      let index = std::sync::Mutex::new(index);
      let node = node_name(&cli)?;
      let blobs = BlobStore::open(&data_dir(&cli)?.join("blobs"))?;
      let ctx = ChatContext {
        node: &node,
        root: &root,
        index: &index,
        blobs: &blobs,
      };
      for (who, text) in transcript(&ctx, &id)? {
        println!("— {who}\n{}\n", text.trim_end());
      }
    }
    Command::LooseEnds { auto, rules, .. } if auto.is_some() || rules.is_some() => {
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        let ask =
          |mode: Option<String>, rules: Option<String>| Request::LooseEndsAuto { mode, rules };
        let said =
          |reply: Reply| -> std::result::Result<(serde_json::Value, Vec<String>), Box<dyn Error>> {
            match reply {
              Reply::Lines(mut l) if !l.is_empty() => {
                let v: serde_json::Value = serde_json::from_str(&l.remove(0))?;
                Ok((v, l))
              }
              Reply::Error(e) => Err(e.into()),
              other => Err(format!("unexpected reply: {other:?}").into()),
            }
          };
        let (now, lines) = said(c.request(&ask(None, None)).await?)?;
        let current = now["rules"].as_str().unwrap_or("").to_owned();
        let new_rules = match rules.as_deref() {
          None | Some("show") => None,
          Some("clear") => Some(String::new()),
          Some("edit") => edited("loose ends' auto-handling", &current)?,
          Some("-") => {
            let mut t = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut t)?;
            Some(t)
          }
          Some(path) => Some(std::fs::read_to_string(path)?),
        }
        .filter(|r| *r != current);
        if auto.is_none() && new_rules.is_none() {
          for l in lines {
            println!("{l}");
          }
          return Ok(());
        }
        let (_, lines) = said(c.request(&ask(auto.clone(), new_rules)).await?)?;
        for l in lines {
          println!("{l}");
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::LooseEnds { list, dismiss, .. } => {
      let check = !list && dismiss.is_none();
      let reply = runtime()?.block_on(async {
        client(&cli)
          .await?
          .request(&Request::LooseEnds {
            check,
            dismiss: dismiss.clone(),
          })
          .await
          .map_err(Box::<dyn Error>::from)
      })?;
      match reply {
        Reply::Lines(lines) => {
          for l in lines {
            println!("{l}");
          }
        }
        Reply::Error(e) => return Err(e.into()),
        other => return Err(format!("unexpected reply: {other:?}").into()),
      }
    }
    Command::Dashboard => {
      let reply = runtime()?.block_on(async {
        client(&cli)
          .await?
          .request(&Request::Dashboard)
          .await
          .map_err(Box::<dyn Error>::from)
      })?;
      let app = match reply {
        Reply::Apps { mut apps, .. } if !apps.is_empty() => apps.remove(0),
        Reply::Error(e) => return Err(e.into()),
        other => return Err(format!("unexpected reply: {other:?}").into()),
      };
      println!(
        "{} is pinned (on your phone too), and may see your threads, jobs and approvals.",
        app.name
      );
      let url = app
        .url
        .ok_or("this node serves no apps to a browser ([apps] port = 0)")?;
      open_in_browser(&url);
    }
    Command::Client {
      no_open,
      thread,
      map,
    } => {
      let (reply, route) = runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        // A thread that is not there is said before a page opens on nothing.
        let route = match thread {
          Some(t) => {
            let id: RecordId = t
              .parse()
              .map_err(|_| format!("`{t}` is not a thread's ID (stitch threads)"))?;
            match c
              .request(&Request::Transcript {
                thread: id.to_string(),
                after: None,
                before: None,
                tail: Some(1),
              })
              .await
            {
              Ok(Reply::Error(_)) | Err(_) => {
                return Err(format!("no thread {id} on this node (stitch threads)").into());
              }
              Ok(_) => format!("#/t/{id}"),
            }
          }
          None if *map => "#/map".to_owned(),
          None => String::new(),
        };
        let reply = c
          .request(&Request::Client)
          .await
          .map_err(Box::<dyn Error>::from)?;
        Ok::<_, Box<dyn Error>>((reply, route))
      })?;
      match reply {
        Reply::App { url: Some(url), .. } => {
          // The token is in the query and the route in the fragment: the
          // node's 303 that drops `?k=` keeps the fragment (the browser
          // carries it over), and the client reads it at start.
          let url = format!("{url}{route}");
          println!("{url}");
          if !*no_open && !launch_browser(&url) {
            eprintln!("(no browser could be started here: open the address above)");
          }
        }
        Reply::App { url: None, .. } => {
          return Err("this node serves no apps to a browser ([apps] port = 0)".into());
        }
        Reply::Error(e) => return Err(e.into()),
        other => return Err(format!("unexpected reply: {other:?}").into()),
      }
    }
    Command::App(AppCommand::Export { what, dir, force }) => {
      let reply = runtime()?.block_on(async {
        client(&cli)
          .await?
          .request(&Request::AppExport { what: what.clone() })
          .await
          .map_err(Box::<dyn Error>::from)
      })?;
      let Reply::AppFiles {
        name,
        version,
        files,
      } = reply
      else {
        return match reply {
          Reply::Error(e) => Err(e.into()),
          other => Err(format!("unexpected reply: {other:?}").into()),
        };
      };
      let dir = dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(stitch_node::apps::slug(&name)));
      let blobs = BlobStore::open(&data_dir(&cli)?.join("blobs"))?;
      let n = stitch_node::apps::export_files(&blobs, &files, &dir, *force)?;
      println!(
        "{name} ({version}): {n} file(s) written to {}",
        dir.display()
      );
    }
    Command::App(action) => {
      let request = match action {
        AppCommand::Export { .. } => unreachable!("handled above"),
        AppCommand::Open { app: Some(what) } => Request::AppOpen { what: what.clone() },
        AppCommand::List | AppCommand::Open { app: None } => Request::Apps,
        AppCommand::Pin { what, name, icon } => Request::AppPin {
          what: what.clone(),
          name: name.clone(),
          icon: icon.clone(),
          fixed: None,
        },
        AppCommand::Unpin { app } => Request::AppUnpin { app: app.clone() },
        AppCommand::AllowLive { app, thread, deny } => Request::AppLive {
          app: app.clone(),
          thread: thread.clone(),
          allow: !deny,
        },
        AppCommand::AllowThreads {
          app,
          thread,
          needs,
          deny,
        } => Request::AppThreads {
          app: app.clone(),
          thread: thread.clone(),
          allow: !deny,
          needs: needs.clone(),
        },
        AppCommand::AllowContainer {
          app,
          thread,
          build,
          run,
          shell,
          deny,
        } => Request::AppContainer {
          app: app.clone(),
          thread: thread.clone(),
          deny: *deny,
          build: build.clone(),
          run: run.clone(),
          shell: shell.clone(),
        },
        AppCommand::Storage { remove } => Request::AppStorage {
          remove: remove.clone(),
        },
      };
      let reply = runtime()?.block_on(async {
        client(&cli)
          .await?
          .request(&request)
          .await
          .map_err(Box::<dyn Error>::from)
      })?;
      let (apps, launcher) = match reply {
        Reply::Apps { apps, launcher } => (apps, launcher),
        Reply::App { name, url } => {
          let url = url.ok_or("this node serves no apps to a browser ([apps] port = 0)")?;
          println!("{name}: {url}");
          open_in_browser(&url);
          return Ok(());
        }
        Reply::Ok => {
          println!("Unpinned.");
          return Ok(());
        }
        Reply::Lines(lines) => {
          for l in lines {
            println!("{l}");
          }
          return Ok(());
        }
        Reply::Error(e) => return Err(e.into()),
        other => return Err(format!("unexpected reply: {other:?}").into()),
      };
      match action {
        AppCommand::Open { .. } => {
          let url = launcher.ok_or("this node serves no apps to a browser ([apps] port = 0)")?;
          open_in_browser(&url);
        }
        AppCommand::Pin { .. } => {
          for a in &apps {
            if a.fixed {
              println!(
                "Pinned {}, at version {}: it stays there.",
                a.name, a.version
              );
            } else if a.thread.is_some() {
              println!(
                "Pinned {}: it follows the newer versions its thread shows.",
                a.name
              );
            } else {
              println!("Pinned {}.", a.name);
            }
            if let Some(u) = &a.url {
              println!("{u}");
            }
          }
        }
        _ => {
          if apps.is_empty() {
            println!("No pinned apps (stitch app pin <thread>).");
          }
          for a in &apps {
            let name = if a.fixed {
              format!("{} (fixed)", a.name)
            } else {
              a.name.clone()
            };
            println!("{name}\t{}", a.url.as_deref().unwrap_or(""));
          }
          if let Some(l) = launcher {
            println!("\nAll of them: {l}");
          }
        }
      }
    }
    Command::Apns(action) => {
      let data = data_dir(&cli)?;
      match action {
        ApnsCommand::Set {
          p8,
          key_id,
          team_id,
          file,
        } => {
          let pem = std::fs::read_to_string(p8)?;
          // Checked before it is kept: a key that signs.
          stitch_node::apns::jwt(&pem, key_id, team_id, 0)?;
          let store = keys::set(
            &data,
            stitch_node::apns::KEY,
            &pem,
            if *file { Store::File } else { Store::Keychain },
          )?;
          let mut c = Config::load(&data)?;
          let mut a = c.apns.take().unwrap_or_default();
          a.key_id = Some(key_id.clone());
          a.team_id = Some(team_id.clone());
          c.apns = Some(a);
          c.save(&data)?;
          println!(
            "This node pushes to your phones now ({}). Try it: stitch apns test",
            if store == Store::File {
              "private file"
            } else {
              "keychain"
            }
          );
        }
        ApnsCommand::Relay { url } => {
          if !url.starts_with("https://") {
            return Err("a relay is an https:// URL".into());
          }
          let mut c = Config::load(&data)?;
          let mut a = c.apns.take().unwrap_or_default();
          a.relay = Some(url.trim_end_matches('/').to_owned());
          c.apns = Some(a);
          c.save(&data)?;
          println!(
            "This node pushes through {url} (a key of its own, when set, comes first). Try it: stitch apns test"
          );
        }
        ApnsCommand::Test => {
          let reply = runtime()?.block_on(async {
            client(&cli)
              .await?
              .request(&Request::ApnsTest)
              .await
              .map_err(Box::<dyn Error>::from)
          })?;
          if let Reply::Lines(lines) = reply {
            for l in lines {
              println!("{l}");
            }
          }
        }
        ApnsCommand::Status => {
          let c = Config::load(&data)?;
          let key = keys::get(&data, stitch_node::apns::KEY, stitch_node::apns::KEY).is_some();
          let a = c.apns.clone().unwrap_or_default();
          match (&a.key_id, &a.team_id, &a.relay) {
            (Some(id), Some(team), _) if key => println!("pushes with key {id} of team {team}"),
            (_, _, Some(url)) => println!("pushes through the relay {url}"),
            _ => println!(
              "pushes through the app's relay {} (a key of this node's: stitch apns set)",
              stitch_node::apns::RELAY
            ),
          }
          let index = open_index(&cli)?;
          let devices = stitch_node::apns::devices(&index);
          if devices.is_empty() {
            println!("no phone has registered yet (it does when the app runs)");
          }
          for d in devices {
            let sealed = if d.key.is_some() {
              ""
            } else {
              ", no push key: an older app"
            };
            println!("{} ({}{sealed})", d.node, d.env);
          }
        }
      }
    }
    Command::Auth(action) => {
      use stitch_node::auth;
      use stitch_providers::oauth::Service;
      let data = data_dir(&cli)?;
      runtime()?.block_on(async {
        match action {
          AuthCommand::Login {provider,file} if provider == "github" => {
            let store = if *file { Store::File } else { Store::Keychain };
            let account = stitch_node::github::login(&data, store, |p| {
              println!("Enter the code {} at {} (valid {} minutes).\nWaiting for sign-in…", p.user_code, p.verification_uri, p.expires_in.as_secs() / 60);
            }).await?;
            println!("Signed in to GitHub as {}{}.", account.login, account.name.as_deref().map(|n| format!(" ({n})")).unwrap_or_default());
          }
          AuthCommand::Login {provider,file} => {
            let service = Service::parse(provider)?;
            let store = auth::login(&data,service,if *file { Store::File } else { Store::Keychain }, |url| {
              println!("Open this URL in your browser:\n{url}\n\nWaiting for sign-in (10 minutes). For a remote node, forward localhost port {}.",service.port());
            }).await?;
            println!("Signed in to {} ({}). This subscription is used over any API key for {}.",service.name(),if store == Store::File {"private file"} else {"keychain"},service.name());
          }
          AuthCommand::Logout {provider} if provider == "github" => {
            stitch_node::github::logout(&data)?;
            println!("Signed out of GitHub here. GitHub keeps the authorization until it is revoked in its settings (Applications).");
          }
          AuthCommand::Logout {provider} => {
            let service = Service::parse(provider)?;
            auth::logout(&data,service).await?;
            println!("Signed out of {}.",service.name());
          }
          AuthCommand::Status {provider} => {
            let services = match provider.as_deref() {Some("github") => vec![],Some(p) => vec![Service::parse(p)?],None => vec![Service::Anthropic,Service::OpenAi,Service::Grok]};
            let config = stitch_node::config::Config::load(&data)?;
            for service in services {
              let billing = config.billing(&data, service.name()).await;
              println!("{}: {}; billed by {billing}",service.name(),auth::status(&data,service).await?);
            }
            if matches!(provider.as_deref(), None | Some("github")) {
              println!("github: {}", stitch_node::github::status(&data));
            }
          }
        }
        Ok::<_,Box<dyn Error>>(())
      })?;
    }
    Command::Key(KeyCommand::Set { provider, file }) => {
      let key = read_secret(&format!("API key for {provider}: "))?;
      let op = SecretOp::Set {
        name: provider.clone(),
        scope: "global".into(),
        value: Some(key),
        device_only: None,
        file: *file,
      };
      secret_request(&cli, op)?;
    }
    Command::Key(KeyCommand::Rm { provider }) => {
      secret_request(
        &cli,
        SecretOp::Remove {
          name: provider.clone(),
          scope: "global".into(),
        },
      )?;
    }
    Command::Secret(cmd) => {
      let op = match cmd {
        SecretCommand::Set {
          name,
          scope,
          device_only,
          sync,
          keep_value,
          file,
        } => SecretOp::Set {
          name: name.clone(),
          scope: scope.scope(),
          value: if *keep_value {
            None
          } else {
            Some(read_secret(&format!("{name}: "))?)
          },
          device_only: match (device_only, sync) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
          },
          file: *file,
        },
        SecretCommand::Get {
          name,
          scope,
          reveal: true,
        } => SecretOp::Get {
          name: name.clone(),
          scope: scope.scope(),
        },
        SecretCommand::Get { .. } | SecretCommand::Ls => SecretOp::List,
        SecretCommand::Rm { name, scope } => SecretOp::Remove {
          name: name.clone(),
          scope: scope.scope(),
        },
      };
      match (cmd, secret_request(&cli, op)?) {
        (SecretCommand::Get { name, scope, .. }, Reply::Secrets(list)) => {
          let want = scope.scope();
          let found = list
            .iter()
            .find(|s| s.name == *name && (want == "here" || s.scope == want));
          match found {
            Some(s) => {
              print_secret(s);
              println!("  (its value: stitch secret get {name} --reveal)");
            }
            None => return Err(format!("no secret `{name}`").into()),
          }
        }
        (_, Reply::Secrets(list)) => {
          if list.is_empty() {
            println!("no secrets: stitch secret set <name>");
          }
          for s in &list {
            print_secret(s);
          }
        }
        _ => {}
      }
    }
    Command::Var(cmd) => {
      let op = match cmd {
        VarCommand::Set { name, value, scope } => SecretOp::VarSet {
          name: name.clone(),
          scope: scope.scope(),
          value: value.clone(),
        },
        VarCommand::Ls => SecretOp::VarList,
        VarCommand::Rm { name, scope } => SecretOp::VarRemove {
          name: name.clone(),
          scope: scope.scope(),
        },
      };
      if let Reply::Vars(list) = secret_request(&cli, op)? {
        if list.is_empty() {
          println!("no variables: stitch var set <name> <value>");
        }
        for v in list {
          let scope = if v.scope == "global" {
            String::new()
          } else {
            format!("  ({})", v.scope_label)
          };
          println!("{} = {}{scope}", v.name, v.value);
        }
      }
    }
    Command::Model(cmd) => {
      let input = match cmd {
        ModelCommand::Ls { node } => serde_json::json!({"action": "ls", "node": node}),
        ModelCommand::Default { model, unset } => {
          serde_json::json!({"action": "default", "model": model, "unset": unset})
        }
        ModelCommand::Quick { model, unset } => {
          serde_json::json!({"action": "quick", "model": model, "unset": unset})
        }
        ModelCommand::Set {
          pattern,
          reserved,
          price,
          note,
        } => serde_json::json!({
          "action": "set", "pattern": pattern, "reserved": reserved,
          "price": price, "note": note,
        }),
        ModelCommand::Rm { pattern } => serde_json::json!({"action": "rm", "pattern": pattern}),
      };
      let req = Request::ModelSettings {
        input: input.to_string(),
      };
      runtime()?.block_on(async {
        match client(&cli).await?.request(&req).await? {
          Reply::Lines(lines) => {
            for l in lines {
              println!("{l}");
            }
            Ok::<_, Box<dyn Error>>(())
          }
          Reply::Error(e) => Err(e.into()),
          other => Err(format!("unexpected reply: {other:?}").into()),
        }
      })?;
    }
    Command::Ssh(cmd) => ssh_command(&cli, cmd)?,
    Command::Invite { minutes } => {
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        if let Reply::Ticket(t) = c
          .request(&Request::Invite {
            ttl_secs: minutes * 60,
          })
          .await?
        {
          print_qr(&t)?;
          println!("pairing ticket (valid {minutes} minutes, once):\n{t}");
          println!("on the new node: stitch join <ticket>");
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Join { ticket, home } => {
      let ticket: Ticket = ticket.parse()?;
      let data = data_dir(&cli)?;
      let name = node_name(&cli)?;
      let restarted = stop_node(&cli)?;
      let dir = home
        .clone()
        .unwrap_or_else(|| stitch_node::setup::default_home(&data));
      let joined = runtime()?.block_on(stitch_node::setup::join(&data, &name, &ticket, &dir))?;
      println!("joined {} ({})", joined.inviter.name, joined.inviter.id);
      for p in &joined.fleet {
        println!("  also trusting {}", p.name);
      }
      match &joined.home {
        Some(report) => print_report(report),
        None => println!("{} holds no home workspace", joined.inviter.name),
      }
      if restarted {
        println!("the node starts again with the next command");
      }
    }
    Command::Sync { peer } => {
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        if let Reply::Synced(lines) = c.request(&Request::Sync { peer: peer.clone() }).await? {
          if lines.is_empty() {
            println!("no peers or no repositories");
          }
          for l in lines {
            println!("{l}");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Id => {
      let key = load_or_create_key(&data_dir(&cli)?.join("node.key"))?;
      println!("{}", key.public());
    }
    Command::Peer(PeerCommand::Add { name, id, addrs }) => {
      let path = data_dir(&cli)?.join("peers.toml");
      let mut peers = Peers::load(&path)?;
      peers.add(Peer {
        name: name.clone(),
        id: id.clone(),
        addrs: addrs.clone(),
      })?;
      std::fs::create_dir_all(data_dir(&cli)?)?;
      peers.save(&path)?;
    }
    Command::Peer(PeerCommand::Ls) => {
      let state = trust(&cli)?;
      // What the running node last knew of each (`peers-seen.toml`).
      let seen = state
        .seen_path()
        .map(|p| stitch_net::SeenPeers::load(&p))
        .unwrap_or_default();
      for p in state.known_peers()? {
        let addrs: Vec<String> = p.addrs.iter().map(ToString::to_string).collect();
        println!("{:<12} {}  {}", p.name, p.id, addrs.join(" "));
        if let Some(s) = seen.peer.get(&p.name) {
          println!("{:<12} {}", "", seen_line(s));
        }
      }
    }
    Command::Peer(PeerCommand::Remove { name }) => {
      let state = trust(&cli)?;
      let target = state
        .find_peer(name)?
        .ok_or_else(|| format!("no peer `{name}`"))?;
      let mut revoked = 0;
      if let Some(home) = state.home() {
        let records = state.index.lock().expect("index lock").peer_records()?;
        for r in records
          .iter()
          .filter(|r| r.fields.get("endpoint").and_then(|v| v.as_str()) == Some(target.id.as_str()))
        {
          let draft = Draft::new(kinds::REVOKE, Author::user(state.node.clone()))?
            .reference("revokes", Ref::to(r.id.clone()))?;
          home.publish(&state.node, draft)?;
          revoked += 1;
        }
      }
      let path = data_dir(&cli)?.join("peers.toml");
      let mut local = Peers::load(&path)?;
      local.peers.retain(|p| p.id != target.id);
      local.save(&path)?;
      let spread = if revoked > 0 {
        "; the revocation spreads with the home workspace"
      } else {
        ""
      };
      println!("{} removed{spread}", target.name);
    }
    Command::Project(ProjectCommand::Ls) => {
      let mut index = open_index(&cli)?;
      all_roots(&mut index)?;
      let mut seen = place::Checkouts::load(&data_dir(&cli)?);
      let places = place::Places::load(&index);
      for p in places.listed() {
        let fork = if p.split { " (split)" } else { "" };
        let remote = p.remote.as_deref().unwrap_or("no remote");
        println!("{}{fork}  {}  {remote}", p.name, p.repo);
        if let Some(dir) = seen.of(&p.repo, &places.projects) {
          println!("  here: {}", dir.display());
        }
      }
    }
    Command::Memory(MemoryCommand::Install { text_only }) => {
      let data = data_dir(&cli)?;
      let mut last = (String::new(), 0u64);
      let files = runtime()?.block_on(stitch_node::memory::install(
        &data,
        !text_only,
        |name, got, total| {
          let mb = got >> 20;
          if last.0 != name || mb >= last.1 + 50 || (total > 0 && got == total) {
            last = (name.to_owned(), mb);
            eprint!("\r{name}: {mb} / {} MB   ", total >> 20);
          }
        },
      ))?;
      eprintln!();
      for f in files {
        println!("installed {}", f.display());
      }
      if stitch_node::running(&data) {
        stop_node(&cli)?;
        println!("memory starts with the node's next start (the next command)");
      }
    }
    Command::Memory(MemoryCommand::Status) => {
      runtime()?.block_on(async {
        let mut c = client(&cli).await?;
        if let Reply::Memory {
          available,
          facts,
          episodes,
          ..
        } = c
          .request(&Request::Memory {
            thread: None,
            cwd: None,
            global: false,
          })
          .await?
        {
          if available {
            println!("memory on: {facts} facts folded, {episodes} episodes");
          } else {
            println!("memory off: run `stitch memory install`");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Memory(MemoryCommand::Notes) => {
      runtime()?.block_on(async {
        if let Reply::Lines(lines) = client(&cli).await?.request(&Request::MemoryNotes).await? {
          for l in lines {
            println!("{l}");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Memory(MemoryCommand::Audit) => {
      runtime()?.block_on(async {
        if let Reply::Lines(lines) = client(&cli).await?.request(&Request::MemoryAudit).await? {
          for l in lines {
            println!("{l}");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Memory(m @ (MemoryCommand::Off { thread } | MemoryCommand::On { thread })) => {
      let on = matches!(m, MemoryCommand::On { .. });
      let req = Request::Remember {
        thread: thread.clone().unwrap_or_else(|| "last".into()),
        on,
        cwd: std::env::current_dir()
          .ok()
          .map(|d| d.display().to_string()),
      };
      runtime()?.block_on(async {
        if let Reply::Lines(lines) = client(&cli).await?.request(&req).await? {
          for l in lines {
            println!("{l}");
          }
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Browser(BrowserCommand::Install { what, profile_dir }) => {
      let data = data_dir(&cli)?;
      match what.as_deref() {
        Some("headless") => {
          println!("downloading headless Chromium…");
          let exe = runtime()?.block_on(stitch_node::web::install(&data))?;
          println!("installed {}", exe.display());
          println!("the web tools use it from the next browser start");
        }
        Some(other) => {
          return Err(format!("install what? `{other}`: nothing, or `headless`").into());
        }
        None => {
          let exe = std::env::current_exe()?;
          let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
          let done = stitch_node::browser::install::install(&data, &exe, &home, profile_dir)?;
          println!("Stitch's extension: {}", done.extension.display());
          if done.manifests.is_empty() {
            println!(
              "No Chromium browser found (Chrome, Chromium, Brave, Edge): start one once, then run this again."
            );
          }
          for (name, path) in &done.manifests {
            println!("native messaging host for {name}: {}", path.display());
          }
          println!(
            "\nIn each Chrome profile Stitch may use:\n  1. open chrome://extensions and switch on Developer mode (top right);\n  2. click \"Load unpacked\" and choose {};\n  3. click the Stitch icon in the toolbar (pin it from the puzzle menu) and switch on \"Allow Stitch in this profile\".\nThen `stitch browser profiles` lists it. After `stitch browser install` again, press the reload button on the extension's card.",
            done.extension.display()
          );
        }
      }
    }
    Command::Browser(BrowserCommand::Profiles { action, name }) => {
      let disable = match (action.as_deref(), name) {
        (None, _) => None,
        (Some("disable"), Some(n)) => Some(n.clone()),
        _ => return Err("usage: stitch browser profiles [disable <name>]".into()),
      };
      runtime()?.block_on(async {
        match client(&cli)
          .await?
          .request(&Request::BrowserProfiles { disable })
          .await?
        {
          Reply::Lines(lines) => lines.iter().for_each(|l| println!("{l}")),
          Reply::Error(e) => return Err(e.into()),
          other => return Err(format!("unexpected reply {other:?}").into()),
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Browser(BrowserCommand::Call {
      tool,
      input,
      thread,
      images,
    }) => {
      let ok = runtime()?.block_on(async {
        let req = Request::BrowserCall {
          thread: thread.clone(),
          tool: tool.clone(),
          input: input.clone(),
          cwd: std::env::current_dir()
            .ok()
            .map(|d| d.display().to_string()),
        };
        match client(&cli).await?.request(&req).await? {
          Reply::ToolDone {
            ok,
            output,
            images: imgs,
          } => {
            println!("{output}");
            let dir = images.clone().unwrap_or_else(std::env::temp_dir);
            for (i, (media, bytes)) in imgs.iter().enumerate() {
              let ext = media.rsplit('/').next().unwrap_or("png");
              let path = dir.join(format!("stitch-browser-{}-{i}.{ext}", std::process::id()));
              std::fs::write(&path, bytes)?;
              println!("[image: {}]", path.display());
            }
            Ok::<_, Box<dyn Error>>(ok)
          }
          Reply::Error(e) => Err(e.into()),
          other => Err(format!("unexpected reply {other:?}").into()),
        }
      })?;
      if !ok {
        std::process::exit(1);
      }
    }
    Command::BrowserHost { args: _ } => {
      let data = data_dir(&cli)?;
      let exe = std::env::current_exe()?;
      runtime()?.block_on(stitch_node::browser::host::run(&data, &exe))?;
    }
    Command::Folder(action) => {
      let mut index = open_index(&cli)?;
      let roots = all_roots(&mut index)?;
      let places = place::Places::load(&index);
      let cwd = std::env::current_dir()?;
      // In a global thread's working folder: that folder (design §2.3).
      let workdirs = data_dir(&cli)?.join(stitch_node::THREAD_FOLDERS);
      let workdirs = std::fs::canonicalize(&workdirs).unwrap_or(workdirs);
      let workdir = std::fs::canonicalize(&cwd).ok().and_then(|c| {
        let origin = c.strip_prefix(&workdirs).ok()?.components().next()?;
        let origin: RecordId = origin.as_os_str().to_str()?.parse().ok()?;
        Some((
          stitch_node::folders::workdir_key(&origin),
          workdirs.join(origin.as_str()),
        ))
      });
      let (key, top) = match workdir {
        Some((key, top)) => (key, Some(top)),
        None => {
          let Place::Folder(key) = here(&cli, &roots, &places)? else {
            return Err(
              "not in a folder: a git checkout is a project, and the home directory is global"
                .into(),
            );
          };
          let top = places.folders.dir(&key, node_name(&cli)?.as_str());
          (key, top)
        }
      };
      // Paths given relative to where this runs, as the folder has them.
      let in_folder = |p: &Option<String>| -> Option<String> {
        let p = p.as_ref()?;
        let abs = cwd.join(p);
        Some(match top.as_ref().and_then(|t| abs.strip_prefix(t).ok()) {
          Some(rel) => rel.display().to_string(),
          None => p.clone(),
        })
      };
      match action {
        FolderCommand::Sync { allow, stop } => {
          if *allow || *stop {
            let node = node_name(&cli)?;
            let home = roots
              .iter()
              .find(|r| r.config().role == Role::Home)
              .ok_or("no home workspace")?;
            home.publish(&node, place::sync_draft(&node, &key, *allow)?)?;
            index.refresh(home)?;
            if *stop {
              println!("{key}: its files stay on each node from now on");
              return Ok(());
            }
          }
          let reply = runtime()?.block_on(async {
            client(&cli)
              .await?
              .request(&Request::FolderSync { key: key.clone() })
              .await
              .map_err(Box::<dyn Error>::from)
          })?;
          let Reply::FolderPass {
            published,
            applied,
            conflicts,
            skipped,
          } = reply
          else {
            return Err(format!("unexpected reply: {reply:?}").into());
          };
          if let Some(why) = skipped {
            println!("not synced: {why}");
          }
          if published {
            println!("this node's changes published");
          }
          if applied > 0 {
            println!("{applied} file(s) from other nodes");
          }
          for c in conflicts {
            println!("edited on two nodes, both kept: {c}");
          }
          return Ok(());
        }
        FolderCommand::Log { path, limit } => {
          for c in stitch_node::folders::log(&index, &key, in_folder(path).as_deref())?
            .into_iter()
            .take(*limit)
          {
            let by = c
              .thread
              .map(|t| format!(", thread {t}"))
              .unwrap_or_default();
            let merge = if c.merge { " (merge)" } else { "" };
            println!(
              "{}  {}  {}{by}{merge}",
              c.id,
              c.at.strftime("%Y-%m-%d %H:%M"),
              c.node
            );
            for p in c.changed.iter().take(10) {
              println!("    {p}");
            }
            if c.changed.len() > 10 {
              println!("    … {} more", c.changed.len() - 10);
            }
          }
          return Ok(());
        }
        FolderCommand::Diff { path, from, to } => {
          let blobs = BlobStore::open(&data_dir(&cli)?.join("blobs"))?;
          let parse = |s: &Option<String>| -> Result<Option<RecordId>> {
            Ok(s.as_deref().map(str::parse).transpose()?)
          };
          print!(
            "{}",
            stitch_node::folders::diff(
              &index,
              &blobs,
              &key,
              parse(from)?.as_ref(),
              parse(to)?.as_ref(),
              in_folder(path).as_deref(),
            )?
          );
          return Ok(());
        }
        FolderCommand::Link { .. } | FolderCommand::Unlink => {}
      }
      let node = node_name(&cli)?;
      let loc = place::location_of(node.as_str(), &std::env::current_dir()?);
      let draft = match action {
        FolderCommand::Link { other, force } => {
          match place::link(&index, &node, &loc, other, *force)? {
            place::Link::Draft(d) => *d,
            place::Link::Same(label) => {
              println!("already the same folder: {label}; nothing to link");
              return Ok(());
            }
          }
        }
        FolderCommand::Unlink => place::unlink_draft(&node, &loc)?,
        _ => unreachable!("answered above"),
      };
      let home = roots
        .iter()
        .find(|r| r.config().role == Role::Home)
        .ok_or("no home workspace")?;
      home.publish(&node, draft)?;
      index.refresh(home)?;
      let places = place::Places::load(&index);
      println!(
        "this folder: {}",
        here(&cli, &roots, &places)?.label(node.as_str(), &places)
      );
    }
    Command::Project(ProjectCommand::Rebind { project }) => {
      let cwd = std::env::current_dir()?.display().to_string();
      let project = project.clone();
      runtime()?.block_on(async {
        let req = Request::Rebind { cwd, project };
        if let Reply::Rebind(said) = client(&cli).await?.request(&req).await? {
          println!("{said}");
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::Project(ProjectCommand::Split) => {
      let cwd = std::env::current_dir()?.display().to_string();
      runtime()?.block_on(async {
        if let Reply::Split(name) = client(&cli).await?.request(&Request::Split { cwd }).await? {
          println!("{name}: new threads here belong to it");
        }
        Ok::<_, Box<dyn Error>>(())
      })?;
    }
    Command::New {
      kind,
      title,
      body,
      refs,
      fields,
      attachments,
      user,
    } => {
      let (root, _) = open(&cli)?;
      let node = node_name(&cli)?;
      let author = if *user {
        Author::user(node.clone())
      } else {
        Author::node(node.clone())
      };
      let mut text = String::new();
      if let Some(t) = title {
        text.push_str(&format!("# {t}\n"));
      }
      if let Some(b) = body {
        let b = if b == "-" { read_stdin()? } else { b.clone() };
        if !text.is_empty() {
          text.push('\n');
        }
        text.push_str(&b);
      }
      let mut draft = Draft::new(kind, author)?.body(text);
      for f in fields {
        let (k, v) = parse_field(f)?;
        draft.set_field(&k, v)?;
      }
      for r in refs {
        let (rel, target) = r
          .split_once('=')
          .ok_or_else(|| format!("`{r}`: expected rel=ref"))?;
        draft = draft.reference(rel, target.parse()?)?;
      }
      if !attachments.is_empty() {
        let blobs = BlobStore::open(&data_dir(&cli)?.join("blobs"))?;
        for path in attachments {
          let (hash, bytes) = blobs.put_file(path)?;
          draft = draft.attach(Attachment {
            hash,
            media_type: stitch_node::media_type_of(path).to_owned(),
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            bytes,
          });
        }
      }
      let record = root.publish(&node, draft)?;
      println!("{}", record.id);
    }
    Command::Show { id } => {
      let (root, index) = open(&cli)?;
      let id: RecordId = id.parse()?;
      let record = root.read_record(&id)?;
      print!("{}", record.to_text());
      println!("--");
      println!("status: {}", status(&index, &id, record.kind.as_str())?);
      if let Some(t) = index.title(&id)? {
        println!("title: {t}");
      }
      for e in index.edges_to(&id)? {
        println!("<- {} {}", e.rel, e.src);
      }
    }
    Command::Ls { kind } => {
      let (_, index) = open(&cli)?;
      for r in index.records(kind.as_deref())? {
        println!(
          "{}  {:<12} {:<12} {}",
          r.id,
          r.kind,
          status(&index, &r.id, &r.kind)?,
          index.title(&r.id)?.unwrap_or_default()
        );
      }
    }
    Command::Attention => {
      let (_, index) = open(&cli)?;
      for item in index.attention()? {
        match item {
          AttentionItem::Question { id, title } => {
            println!(
              "question      {id}  {}",
              tui::question_line(&index, &id, title)
            );
            let grant: Vec<String> = index
              .record(&id)?
              .and_then(|r| r.fields.get("grant").and_then(|g| g.as_array()).cloned())
              .unwrap_or_default()
              .iter()
              .filter_map(|g| g.as_str().map(|g| format!("`{g}`")))
              .collect();
            if grant.is_empty() {
              println!("              stitch answer {id} y|n [why]");
            } else {
              println!(
                "              stitch answer {id} y|n [why], or always: {} run without asking there from then on",
                grant.join(", ")
              );
            }
          }
          AttentionItem::Finding { finding } => println!("still failing {finding}"),
          AttentionItem::OverBudget { task } => println!("over budget   {task}"),
        }
      }
    }
    Command::Edges => {
      let (_, index) = open(&cli)?;
      for e in index.edges()? {
        let derived = if e.derived { "\t(derived)" } else { "" };
        println!("{}\t{}\t{}{derived}", e.src, e.rel, e.dst);
      }
    }
    Command::SyncLocal { other } => {
      let (root, mut index) = open(&cli)?;
      let other_root = RecordRoot::open(other)?;
      // The other root belongs to another node, which keeps its own index.
      let mut other_index = Index::open_in_memory()?;
      other_index.refresh(&other_root)?;
      let report = sync_roots(
        &root,
        &index.items(root.repo())?,
        &other_root,
        &other_index.items(other_root.repo())?,
      )?;
      println!(
        "{} in, {} out, {} rounds, {} messages",
        report.into_a.len(),
        report.into_b.len(),
        report.exchange.rounds,
        report.exchange.messages
      );
      for c in &report.conflicts {
        eprintln!("conflict: {c}");
      }
      index.refresh(&root)?;
    }
    Command::Reindex => {
      let dir = data_dir(&cli)?;
      // The roots the index knew (the home workspace may live anywhere),
      // `--root`, and the projects' stores where they live.
      let old = open_index(&cli);
      let mut dirs: Vec<PathBuf> = old
        .as_ref()
        .ok()
        .and_then(|i| i.roots().ok())
        .unwrap_or_default()
        .into_values()
        .map(PathBuf::from)
        .collect();
      dirs.extend(cli.root.clone());
      if let Ok(entries) = std::fs::read_dir(dir.join("projects")) {
        dirs.extend(entries.flatten().map(|e| e.path()));
      }
      // Emptied in place, through SQLite: the node and any TUI keep the
      // file open, and deleting it (its WAL and shared memory) under them
      // corrupts what they read, or crashes them.
      let mut index = match old.and_then(|mut i| {
        i.reset()?;
        Ok(i)
      }) {
        Ok(i) => i,
        // A file SQLite cannot use any more is deleted, but only with no
        // node running (a TUI starts one, so none is open either).
        Err(e) if stitch_node::running(&dir) => {
          return Err(
            format!(
              "the index could not be emptied in place ({e}); stop the node \
               (`stitch node stop`) and close every TUI, then run `stitch reindex` again"
            )
            .into(),
          );
        }
        Err(e) => {
          eprintln!("the index could not be emptied in place ({e}): deleting it");
          let path = dir.join("index.sqlite");
          for suffix in ["", "-wal", "-shm"] {
            let p = PathBuf::from(format!("{}{suffix}", path.display()));
            if p.exists() {
              std::fs::remove_file(&p)?;
            }
          }
          open_index(&cli)?
        }
      };
      for d in dirs {
        if let Ok(r) = RecordRoot::open(&d) {
          index.refresh(&r)?;
        }
      }
      println!("{} records", index.records(None)?.len());
    }
  }
  Ok(())
}

/// A peer's reachability: `reached 2m ago, direct 192.168.0.20:4433, 3 ms;
/// last sync 0.81s`, then the failure since, if any.
fn seen_line(s: &stitch_net::Seen) -> String {
  let mut parts = Vec::new();
  match s.reached {
    Some(at) => {
      let mut p = format!("reached {}", stitch_net::ago(at));
      match s.path {
        Some(stitch_net::PathKind::Direct) => {
          p.push_str(", direct");
          if let Some(a) = &s.addr {
            p.push_str(&format!(" {a}"));
          }
        }
        Some(stitch_net::PathKind::Relay) => p.push_str(", relay"),
        None => {}
      }
      if let Some(ms) = s.rtt_ms {
        p.push_str(&format!(", {ms} ms"));
      }
      parts.push(p);
    }
    None => parts.push("never reached".into()),
  }
  if let Some(ms) = s.sync_ms {
    parts.push(format!("last sync {:.2}s", ms as f64 / 1000.0));
  }
  if let (Some(at), Some(e)) = (s.failed, &s.error) {
    parts.push(format!("failed {}: {e}", stitch_net::ago(at)));
  }
  parts.join("; ")
}

/// The trust view (local list and peer records) without a running node.
fn trust(cli: &Cli) -> Result<NodeState> {
  let data = data_dir(cli)?;
  let index = open_index(cli)?;
  let mut roots = std::collections::BTreeMap::new();
  if let Some(home) = home_root(&index)? {
    roots.insert(home.repo().clone(), home);
  }
  let peers_path = data.join("peers.toml");
  Ok(NodeState::new(
    node_name(cli)?,
    roots,
    index,
    BlobStore::open(&data.join("blobs"))?,
    Peers::load(&peers_path)?,
    Some(peers_path),
    Policy {
      blobs: BlobPolicy::Lazy,
    },
  ))
}

fn main() -> ExitCode {
  match run(Cli::parse()) {
    Ok(()) => ExitCode::SUCCESS,
    Err(e) => {
      eprintln!("stitch: {e}");
      ExitCode::FAILURE
    }
  }
}

/// A tool or skill set's items, one line each: state, name, where it runs
/// and what it is.
fn set_lines(items: &[stitch_node::api::SetItem]) -> Vec<String> {
  let width = items.iter().map(|i| i.name.len()).max().unwrap_or(0);
  items
    .iter()
    .map(|i| {
      let state = match (i.on, i.available) {
        (true, true) => "on ",
        (false, _) => "off",
        (true, false) => "on, unavailable here:",
      };
      let at = i
        .node
        .as_ref()
        .map(|n| format!(" (on {n})"))
        .unwrap_or_default();
      format!("  {state} {:<width$}{at}  {}", i.name, i.note)
    })
    .collect()
}

/// `text` edited in `$EDITOR` (`vi` when unset), in a file of its own;
/// `None` when the editor failed.
fn edited(place: &str, text: &str) -> Result<Option<String>> {
  let path = std::env::temp_dir().join(format!("stitch-instructions-{}.md", std::process::id()));
  std::fs::write(&path, text)?;
  let editor = std::env::var("VISUAL")
    .or_else(|_| std::env::var("EDITOR"))
    .unwrap_or_else(|_| "vi".into());
  eprintln!("Editing your instructions for {place}…");
  // `$EDITOR` may carry its own arguments (`code --wait`).
  let status = std::process::Command::new("sh")
    .arg("-c")
    .arg(format!("{editor} \"$1\""))
    .arg("editor")
    .arg(&path)
    .status()?;
  let out = std::fs::read_to_string(&path);
  let _ = std::fs::remove_file(&path);
  Ok(status.success().then_some(out?))
}

/// `stitch event`.
fn event_command(cli: &Cli, action: &EventCommand) -> Result<()> {
  let print_state = |st: &stitch_node::api::EventState| -> i32 {
    match st.state.as_str() {
      "answered" => {
        println!("{}", st.answer.as_deref().unwrap_or("null"));
        0
      }
      "published" | "pending" => {
        println!(
          "{} {} (matched {} subscription(s))",
          st.id, st.state, st.matched
        );
        0
      }
      other => {
        eprintln!(
          "{} {other}{}",
          st.id,
          st.note
            .as_deref()
            .map(|n| format!(": {n}"))
            .unwrap_or_default()
        );
        2
      }
    }
  };
  let request = match action {
    EventCommand::Publish {
      topic,
      payload,
      ask,
      schema,
      no_wait,
    } => {
      let payload = match payload.as_deref() {
        Some("-") => {
          let mut t = String::new();
          std::io::Read::read_to_string(&mut std::io::stdin(), &mut t)?;
          t
        }
        Some(p) => p.to_owned(),
        None => String::new(),
      };
      Request::Event {
        topic: topic.clone(),
        payload,
        ask: ask.map(|secs| stitch_node::api::EventAsk {
          deadline_ms: (secs * 1000.0).max(1.0) as u64,
          schema: schema.clone(),
        }),
        wait: !no_wait,
      }
    }
    EventCommand::Status { id, wait } => Request::EventStatus {
      id: id.clone(),
      wait_ms: (wait * 1000.0).max(0.0) as u64,
    },
    EventCommand::Ls { topic, limit } => Request::Events {
      input: serde_json::json!({"action": "ls", "topic": topic, "limit": limit}).to_string(),
      cwd: None,
    },
    EventCommand::Subscribe {
      pattern,
      thread,
      schedule,
      filter,
      filter_file,
      debounce,
      max_per_hour,
      daily_usd,
      until,
      note,
      profile,
      answers,
    } => {
      let i = serde_json::json!({
        "action": "subscribe", "topic": pattern, "thread": thread, "schedule": schedule,
        "filter": filter, "filter_file": filter_file, "debounce_secs": debounce,
        "max_per_hour": max_per_hour, "daily_usd": daily_usd, "until": until, "note": note,
        "profile": profile.value(),
        "answers": answers.then_some(true),
      });
      let i: serde_json::Map<String, serde_json::Value> = i
        .as_object()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, v)| !v.is_null())
        .collect();
      Request::Events {
        input: serde_json::Value::Object(i).to_string(),
        cwd: std::env::current_dir()
          .ok()
          .map(|d| d.display().to_string()),
      }
    }
    EventCommand::Unsubscribe { subscription } => Request::Events {
      input: serde_json::json!({"action": "unsubscribe", "subscription": subscription}).to_string(),
      cwd: None,
    },
    EventCommand::Untaint { thread } => Request::Events {
      input: serde_json::json!({"action": "untaint", "thread": thread}).to_string(),
      cwd: None,
    },
  };
  let code = runtime()?.block_on(async {
    match client(cli).await?.request(&request).await? {
      Reply::Event(st) => Ok::<_, Box<dyn Error>>(print_state(&st)),
      Reply::Lines(lines) => {
        for l in lines {
          println!("{l}");
        }
        Ok(0)
      }
      Reply::Error(e) => Err(e.into()),
      other => Err(format!("unexpected reply: {other:?}").into()),
    }
  })?;
  if code != 0 {
    std::process::exit(code);
  }
  Ok(())
}
