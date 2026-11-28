//! `config.toml` in the data directory.
//!
//! ```toml
//! node = "mbp"
//! discovery = "n0"            # or "local": direct addresses only
//! default_model = "anthropic/claude-opus-5-5"   # over the synced one
//! quick_model = "anthropic/claude-sonnet-5-5"   # likewise
//!
//! [providers.ollama]
//! kind = "openai"
//! base_url = "http://localhost:11434/v1"
//! ```
//!
//! A model is named `provider/model`, the provider being a key of
//! `[providers]`. `anthropic` exists without being configured.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use stitch_providers::{Anthropic, Gemini, OpenAiCompat, Provider, Responses};

use crate::{Error, Result, keys};

/// The model of a node with no default set and no provider with credentials
/// (`modelset::BUILTIN` picks one when there is).
pub const DEFAULT_MODEL: &str = "anthropic/claude-opus-5-5";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
  /// This node's name.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub node: Option<String>,
  /// `n0` (relays and DNS discovery) or `local`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub discovery: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub port: Option<u16>,
  /// Seconds between syncs with every known peer; 0 turns it off.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub sync_interval_secs: Option<u64>,
  /// `all` (pull every referenced blob) or `lazy` (on demand).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub blobs: Option<String>,
  /// This node's default model: over the synced one (a project's still
  /// wins in its place).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub default_model: Option<String>,
  /// This node's quick model (`spawn` with `model: "quick"`): over the
  /// synced one, as `default_model` is.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub quick_model: Option<String>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub providers: BTreeMap<String, ProviderConfig>,
  /// The root agent's shell.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub shell: Option<ShellSettings>,
  /// The web tools' browser on this node.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub browser: Option<BrowserSettings>,
  /// The web search behind `search_web`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub search: Option<SearchSettings>,
  /// Memory (engram over EmbeddingGemma 2).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub memory: Option<MemorySettings>,
  /// Chats the user is in.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub chat: Option<ChatSettings>,
  /// Detached threads.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detached: Option<DetachedSettings>,
  /// Tools from MCP servers (`crate::mcp`).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mcp: Option<crate::mcp::McpSettings>,
  /// Summaries, titles and compaction of threads.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub threads: Option<ThreadSettings>,
  /// Schedules (design §2.3): the node a schedule made here is pinned to
  /// when it names none (a phone's go to a desktop).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub schedules: Option<ScheduleSettings>,
  /// Push notifications to the user's phones (`stitch apns set`).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub apns: Option<ApnsSettings>,
  /// Notifications on this machine's own desktop (`crate::toasts`).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub notifications: Option<NotificationSettings>,
  /// Apps in a browser on this machine (design §2.10).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub apps: Option<AppsSettings>,
  /// Tools written in Otter (design §2.17).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub otter: Option<crate::otter::OtterSettings>,
  /// `generate_image` (design §2.8, Images).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub images: Option<ImagesSettings>,
  /// Folder sync's limits (design §2.16).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub folders: Option<FolderSettings>,
  /// Stitch fixing itself (docs/self-improve.md §4): the fixer node, its
  /// budgets and the harness-issues schedule's cadence.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub self_improve: Option<SelfImproveSettings>,
  /// Loose ends (design §2.10): the hourly pass that finds the
  /// conversations waiting on the user.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub loose_ends: Option<LooseEndsSettings>,
  /// Digests (design §2.25): the per-thread summaries the quick model keeps.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub digests: Option<DigestSettings>,
  /// What every shell on this node gets as environment variables, before
  /// a place's own `[env]` (design §2.19): `GITHUB_TOKEN =
  /// "secret:github"`, `"var:<name>"`, or a value as written.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub env: BTreeMap<String, String>,
}

/// `[notifications]`: what this node shows on its own desktop, beside
/// what it pushes to phones (design §2.3, "Notices").
///
/// ```toml
/// [notifications]
/// desktop = true   # default: on for the user's node (`~/.stitch`), off
///                  # for any other data directory (tests, throwaways)
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub desktop: Option<bool>,
}

/// `[folders]`: how much of a folder this node syncs. A folder over
/// either limit syncs nothing, and says so.
///
/// ```toml
/// [folders]
/// max_files = 2000   # files it would carry (default 2000)
/// max_mb = 250       # their size together, in MB (default 250)
/// max_file_mb = 25   # one file, in MB: a larger one stays (default 25)
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_files: Option<usize>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_mb: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_file_mb: Option<u64>,
}

/// `[loose_ends]`: the pass that judges which conversations wait on the
/// user (design §2.10, "Loose ends").
///
/// ```toml
/// [loose_ends]
/// every_minutes = 60   # how often its free check runs (0: never on a timer)
/// push = true          # a push when a conversation newly waits on you
/// model = "quick"      # the model that judges (default: the quick one)
/// auto = "off"         # auto-handling on this node, whatever the user's
///                      # synced setting says: "off", "dry-run" or "on"
///                      # (default: the synced setting, itself off)
/// decide = true        # the decision model judges first (design §2.29)
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LooseEndsSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub every_minutes: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub push: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub auto: Option<String>,
  /// Whether the decision model judges first (design §2.29, the
  /// `loose-ends` profile; default true): the quick model only when it
  /// is unsure or unavailable, or auto-handling is on.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub decide: Option<bool>,
}

/// `[digests]`: the summary kept of each thread, updated after its turns
/// (design §2.25, step 2).
///
/// ```toml
/// [digests]
/// enabled = true            # false: no digests are written on this node
/// model = "quick"           # default: the quick model, as titles take it
/// daily_cap_usd = 2.0       # no more updates today once they cost this
/// backfill = true           # the hourly sweep over threads with none or a stale one
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub enabled: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub daily_cap_usd: Option<f64>,
  /// The sweep that digests what has none or a stale one (default true).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub backfill: Option<bool>,
  /// Seconds between two threads of a sweep (default 5).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub backfill_pause_secs: Option<u64>,
}

// `Config` is `Eq`; a cap is never NaN in a file that parsed as TOML floats
// meant as money.
impl Eq for DigestSettings {}

/// `[images]`: the model behind `generate_image`.
///
/// ```toml
/// [images]
/// model = "gemini/gemini-3-pro-image"   # default: gemini/gemini-3.1-flash-image
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagesSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
}

/// `[apps]`: where this node serves apps to a browser on this machine.
///
/// ```toml
/// [apps]
/// port = 7177   # 0: not served
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppsSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub port: Option<u16>,
}

/// `[apns]`: how this node pushes to the user's phones (design §5,
/// milestone 4): with the app team's key itself (a build signed by the
/// user's own team), or through a relay that holds it (the store's app).
///
/// ```toml
/// [apns]
/// key_id = "AB12CD34EF"        # with a key (`stitch apns set`): its ID
/// team_id = "ABCDE12345"       # and its team's
/// relay = "https://stitch-relay.example.workers.dev"  # else through a relay
/// topic = "com.fragcolor.stitch"  # the app (default)
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApnsSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub key_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub team_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub relay: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub topic: Option<String>,
}

/// `[threads]`: summaries, titles and compaction.
///
/// ```toml
/// [threads]
/// summary_model = "anthropic/claude-sonnet-5-5"  # summarize and compact (default: quick)
/// title_model = "anthropic/claude-haiku-5-5"     # titles (default: quick)
/// titles = true       # false: no generated titles
/// compact_at = 80     # a thread past this percent of its model's context window
///                     # continues in a new thread, summarized; 0 turns it off
/// compact_keep = 10   # messages kept verbatim after the summary
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub summary_model: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title_model: Option<String>,
  /// `false`: new chats keep their first line as title (no model call).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub titles: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub compact_at: Option<u8>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub compact_keep: Option<usize>,
}

/// `[chat]`: how the chats the user is in run.
///
/// ```toml
/// [chat]
/// max_steps = 200  # model calls a turn takes before it asks the user for
///                  # as many again (default 200)
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_steps: Option<u64>,
}

/// `[detached]`: how detached threads run.
///
/// ```toml
/// [detached]
/// max_steps = 200      # model calls a turn takes at most (default 50);
///                      # a spawn can set its own
/// keep_shells = false  # true: a thread's shells outlive its turn
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachedSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_steps: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub keep_shells: Option<bool>,
  /// Reports for an idle thread are gathered until none came for this
  /// many seconds (default 5, at most a minute of gathering in all), then
  /// wake it once, together (design §2.7).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub gather_secs: Option<u64>,
}

/// `[schedules]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleSettings {
  /// The node schedules made here fire on when they name none; else this
  /// node (a phone, which cannot wake on its own, needs one).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub default_node: Option<String>,
  /// How long a check may run, in seconds (default 120).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub check_secs: Option<u64>,
}

/// `[self_improve]` (docs/self-improve.md §4, 6):
///
/// ```toml
/// [self_improve]
/// node = "tower"            # the fixer node: fixes run there
/// place = "~/devel/stitch"  # the Stitch checkout on it
/// budget_usd = 5.0          # each fix
/// daily_usd = 20.0          # all fixes and triage in a day
/// every = "0 */6 * * *"     # the harness-issues schedule, on the fixer node
/// model = "anthropic/…"     # the fixers' model (default: the node's)
/// mode = "ask"              # fixers' and the triage's approvals: "ask", or
///                           # "auto" (the hazard guard lets what it clears
///                           # run; design §2.28)
/// auto_land = false         # true: a protected fix the landing guard clears
///                           # lands with a notice and a revert, unasked
/// # The repositories fixes go to (docs/self-improve.md §6), each with
/// # its checkout on the fixer node and its own mirror, CI, protected
/// # paths and recipes. The first is the core, whose build the nodes run
/// # (or the one named `stitch`); one with `core` builds against that one
/// # and pins it (`pin`, a file whose first line is the core's commit).
/// # Unset: `stitch` alone, at `place` (or ~/devel/stitch). The synced
/// # settings' `places` (`stitch issue settings --places`) win over these.
/// places = [
///   { name = "stitch", path = "~/devel/stitch" },
///   { name = "app", path = "~/devel/app", core = "stitch",
///     pin = "core.pin", areas = ["ios", "app", "testflight"] },
/// ]
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SelfImproveSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub node: Option<String>,
  /// The core's checkout (one place, `stitch`; with `places`, the path of
  /// the core's when that one names none).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub place: Option<String>,
  /// The repositories, when more than the core (see above).
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub places: Vec<RepoPlace>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub budget_usd: Option<f64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub daily_usd: Option<f64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub every: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  /// Origin's URL for the node's mirror of the repository, where it lands
  /// and pushes for fixers (`mirror`); unset: the fixer checkout's own
  /// `origin`, read once when the mirror is made.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub remote: Option<String>,
  /// The fixers' approval mode: `ask` (the default) or `auto`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode: Option<String>,
  /// Whether a protected fix the landing guard clears lands unasked.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub auto_land: Option<bool>,
}

// Budgets are never NaN (TOML has no NaN literal a user would write for
// money), so the config compares as a whole.
impl Eq for SelfImproveSettings {}

/// One repository of `[self_improve] places`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoPlace {
  /// Its name: what issues say (`repo`), its mirror's
  /// (`self-improve/<name>.git`), `--repo` on the command line.
  pub name: String,
  /// Its checkout on the fixer node.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub path: Option<String>,
  /// Origin's URL; unset: the checkout's own `origin`, read once.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub remote: Option<String>,
  /// The place it builds against (and pins): a fix in both lands that one
  /// first.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub core: Option<String>,
  /// The file holding the pinned core commit (its first line).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pin: Option<String>,
  /// An issue's `area` words that send it here.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub areas: Vec<String>,
}

/// `[memory]`: on when its model is installed (`stitch memory install`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySettings {
  /// `false` turns memory off.
  pub enabled: Option<bool>,
  /// `false`: remember text only (the vision projector stays unloaded).
  pub images: Option<bool>,
  /// `false`: encode on the CPU.
  pub gpu: Option<bool>,
  /// The longest text encoded, in tokens (longer is cut). The encoder's
  /// memory grows with it: 8192 (about 1.9 GB) by default, 2048 (about
  /// 700 MB) on iOS and visionOS.
  pub context: Option<u32>,
  /// `provider/model` that condenses quiet memories into gists after a
  /// sleep; the quick model when unset. Also takes `default` and `quick`.
  pub gist_model: Option<String>,
}

/// `[browser]`: the headless Chromium the web tools drive.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSettings {
  /// `false` turns `browse_web` and `web_eval` off.
  pub enabled: Option<bool>,
  /// A Chrome or Chromium executable (default: the one `stitch browser
  /// install` put in the data directory, else an installed Chrome,
  /// Chromium, Edge or Brave).
  pub chrome: Option<std::path::PathBuf>,
  /// Minutes the browser stays running unused (default 5).
  pub idle_minutes: Option<u64>,
  /// The user's own Chrome (design §2.18): sites where agents act without
  /// asking even when nobody watches (`example.com`, `*.example.com`).
  pub sites: Option<Vec<String>>,
  /// Sites where every change asks, and nothing happens unwatched (banks,
  /// health, government, admin consoles).
  pub sensitive: Option<Vec<String>>,
  /// Sites agents never open.
  pub blocked: Option<Vec<String>>,
  /// Seconds without the user's input in a tab before the agent resumes
  /// there (default 4).
  pub resume_after_secs: Option<u64>,
  /// Sites granted per place (`[browser.places."<place>"] sites`), grown by
  /// "always" answers to questions asked in the browser.
  #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
  pub places: std::collections::BTreeMap<String, BrowserPlace>,
}

/// `[browser.places."<place>"]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserPlace {
  #[serde(default)]
  pub sites: Vec<String>,
}

/// `[search]`: `provider = "brave"` (default) or `"perplexity"`, with its
/// key stored by `stitch key set brave` (or `perplexity`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchSettings {
  pub provider: Option<String>,
}

/// `[shell]` in config.toml.
///
/// ```toml
/// [shell]
/// approval = "sandbox"    # the default where shells can be sandboxed (macOS,
///                         # Linux with Landlock): they write only in the
///                         # thread's place, temp and build caches, and only
///                         # what reaches beyond asks; "ask": what the rules
///                         # do not allow waits for the user; "auto":
///                         # yolo with the hazard guard (design §2.28); "allow":
///                         # everything runs; "off": no shell
/// allow = ["cargo test"]  # command prefixes added to the defaults
/// writable = ["~/devel/tmp"]  # more directories sandboxed shells write
/// read_only_allow = ["tokei"]  # what read-only threads run besides the
///                         # built-in read-only commands (`allow` and the
///                         # places' lists do not apply to them)
/// user_shell = false      # true: the user's bash or zsh with their rc
///
/// [shell.places."project:p-42ada59c2195414f"]
/// allow = ["cargo test"]  # added there by "always" answers (or by hand)
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub approval: Option<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub allow: Vec<String>,
  /// Directories sandboxed shells write besides their own (`~` expands).
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub writable: Vec<String>,
  /// Prefixes a read-only thread runs besides the built-in read-only
  /// ones (`allow` and the places' do not apply to it); agents and
  /// writing commands stay refused whatever it says.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub read_only_allow: Vec<String>,
  /// Prefixes allowed in one place only (a project, a folder, `global`),
  /// by its key.
  #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
  pub places: std::collections::BTreeMap<String, PlaceShell>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub user_shell: Option<bool>,
}

/// `[shell.places."<place>"]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceShell {
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub allow: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
  /// `anthropic`, `openai` (Chat Completions), `responses`, or `gemini`
  /// (Google's native API).
  pub kind: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub base_url: Option<String>,
  /// Anthropic: `low` to `max`; Gemini: its `thinkingLevel` (`minimal`
  /// to `high`, as the model takes); empty leaves the model's default.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub effort: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub thinking: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub fallbacks: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache: Option<bool>,
  /// How long cached prompts last. Anthropic: `5m` (the default) or `1h`,
  /// for threads whose turns are more than five minutes apart (writes cost
  /// twice input instead of 1.25 times). OpenAI's API (`responses` or
  /// `openai` kinds): `30m`, at least that long, on GPT-5.6 and later.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache_ttl: Option<String>,
  /// Tokens its models take in at most (for compaction, `[threads]`);
  /// when unset, what the provider says of the model, else a guess by
  /// its name.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub context_window: Option<u64>,
}

/// Warnings for the keys of config text `text` that [`Config`] does not
/// know (they are ignored, not errors: configs may be older or newer than
/// this node), each naming its full path.
pub fn unknown_key_warnings(text: &str) -> Vec<String> {
  let mut paths = Vec::new();
  let de = toml::Deserializer::parse(text);
  let Ok(de) = de else { return Vec::new() };
  if serde_ignored::deserialize(de, |p| paths.push(p.to_string()))
    .map(|_: Config| ())
    .is_err()
  {
    return Vec::new();
  }
  // Names of Config's own (top-level) keys, from its serialized default plus
  // the optional ones that default to absent.
  let known: std::collections::BTreeSet<String> = toml::Table::try_from(Config::default())
    .map(|t| t.keys().cloned().collect())
    .unwrap_or_default();
  paths
    .into_iter()
    .map(|path| path.replace(".?", "")) // serde_ignored's mark for an Option
    .map(|path| {
      let last = path.rsplit('.').next().unwrap_or(&path);
      if path.contains('.') && (known.contains(last) || TOP_LEVEL_HINTS.contains(&last)) {
        format!(
          "config: unknown key `{path}` is ignored; `{last}` is a top-level key, so it probably belongs at the top level, before the first [section]"
        )
      } else {
        format!("config: unknown key `{path}` is ignored")
      }
    })
    .collect()
}

/// Top-level keys that serialize to nothing when unset (so are not in the
/// default's table).
const TOP_LEVEL_HINTS: &[&str] = &["default_model", "quick_model"];

impl Config {
  /// The configuration in `data`, its `[shards]` converted to `[otter]`
  /// first (saved, once).
  pub fn load(data: &Path) -> Result<Config> {
    let path = data.join("config.toml");
    match std::fs::read_to_string(&path) {
      Ok(mut text) => {
        if let Some(converted) = convert_shards(&text) {
          let tmp = data.join("config.toml.tmp");
          std::fs::write(&tmp, &converted).map_err(Error::io(tmp.display()))?;
          std::fs::rename(&tmp, &path).map_err(Error::io(path.display()))?;
          eprintln!("config: [shards] in {} is now [otter]", path.display());
          text = converted;
        }
        for w in unknown_key_warnings(&text) {
          eprintln!("{w} ({})", path.display());
        }
        toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
      }
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
      Err(e) => Err(Error::io(path.display())(e)),
    }
  }

  pub fn save(&self, data: &Path) -> Result<()> {
    std::fs::create_dir_all(data).map_err(Error::io(data.display()))?;
    let path = data.join("config.toml");
    let text = toml::to_string(self).map_err(|e| Error::Config(e.to_string()))?;
    let tmp = data.join("config.toml.tmp");
    std::fs::write(&tmp, text).map_err(Error::io(tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(Error::io(path.display()))
  }

  pub fn node_name(&self) -> Result<stitch_records::NodeName> {
    let name = self.node.as_deref().ok_or_else(|| {
      Error::Config("this node has no name: run `stitch setup --name <name>`".into())
    })?;
    Ok(name.parse()?)
  }

  pub fn provider_config(&self, name: &str) -> Option<ProviderConfig> {
    match self.providers.get(name) {
      Some(p) => Some(p.clone()),
      None if name == "anthropic" => Some(ProviderConfig {
        kind: "anthropic".into(),
        ..ProviderConfig::default()
      }),
      None if name == "openai" || name == "grok" => Some(ProviderConfig {
        kind: "responses".into(),
        base_url: Some(
          if name == "openai" {
            "https://api.openai.com/v1"
          } else {
            "https://api.x.ai/v1"
          }
          .into(),
        ),
        ..ProviderConfig::default()
      }),
      // Moonshot's Kimi on its Anthropic-compatible endpoint (design
      // §2.8, Kimi); China's is `https://api.moonshot.cn/anthropic`.
      None if name == "kimi" => Some(ProviderConfig {
        kind: "anthropic".into(),
        base_url: Some(stitch_providers::MOONSHOT_URL.into()),
        ..ProviderConfig::default()
      }),
      // DeepSeek on Chat Completions, its reasoning sent back (§2.8).
      None if name == "deepseek" => Some(ProviderConfig {
        kind: "openai".into(),
        base_url: Some(stitch_providers::DEEPSEEK_URL.into()),
        ..ProviderConfig::default()
      }),
      // OpenRouter on Chat Completions, any of its models by ID
      // (`openrouter/anthropic/claude-sonnet-5.5`; §2.8, OpenRouter).
      None if name == "openrouter" => Some(ProviderConfig {
        kind: "openai".into(),
        base_url: Some(stitch_providers::OPENROUTER_URL.into()),
        ..ProviderConfig::default()
      }),
      None if name == "gemini" => Some(ProviderConfig {
        kind: "gemini".into(),
        base_url: Some(stitch_providers::gemini::BASE_URL.into()),
        ..ProviderConfig::default()
      }),
      // The Google Antigravity plan, through Google's `agy` CLI (§2.8,
      // Antigravity).
      None if name == ANTIGRAVITY => Some(ProviderConfig {
        kind: ANTIGRAVITY.into(),
        ..ProviderConfig::default()
      }),
      None => None,
    }
  }

  /// Resolve subscription credentials only for the built-in provider's exact
  /// official endpoint. A subscription signed in to wins over an API key,
  /// stored or in the environment (design §2.8); custom endpoints never
  /// inherit OAuth.
  pub async fn resolve_authenticated(
    &self,
    data: &Path,
    model: Option<&str>,
  ) -> Result<(Provider, String, String)> {
    use stitch_providers::oauth::Service;
    let (mut provider, name, model) = self.resolve(data, model)?;
    let cfg = self.provider_config(&name).expect("resolved provider");
    let service = subscription_service(&name, &cfg);
    if let Some(service) = service
      && let Some(c) = crate::auth::credentials(data, service).await?
    {
      // The token read here starts the turn; the provider asks this source
      // again before each request, so a long turn outlives it.
      let tokens = crate::auth::tokens(data, service);
      match service {
        Service::Anthropic => {
          if let Provider::Anthropic(p) = &mut provider {
            p.api_key = c.access_token;
            p.oauth = true;
            p.tokens = Some(tokens);
          }
        }
        Service::OpenAi => {
          let mut p = Responses::codex(
            c.access_token,
            c.account_id
              .ok_or_else(|| Error::Config("OAuth account ID missing; sign in again".into()))?,
          );
          p.effort = cfg.effort;
          p.tokens = Some(tokens);
          provider = Provider::Responses(p);
        }
        Service::Grok => {
          // Keep subscription inference on the subscription proxy. Do not
          // silently fall back to API billing after an entitlement failure.
          match &mut provider {
            Provider::Responses(p) => {
              p.base_url = "https://cli-chat-proxy.grok.com/v1".into();
              p.api_key = c.access_token;
              p.grok = true;
              p.tokens = Some(tokens);
            }
            Provider::OpenAi(p) => {
              p.base_url = "https://cli-chat-proxy.grok.com/v1".into();
              p.api_key = Some(c.access_token);
              p.grok = true;
              p.tokens = Some(tokens);
            }
            _ => unreachable!(),
          }
        }
      }
    }
    Ok((provider, name, model))
  }

  /// How `provider`'s calls are billed here, as `resolve_authenticated`
  /// picks: `subscription`, `API key`, or `none`.
  pub async fn billing(&self, data: &Path, provider: &str) -> &'static str {
    let Some(cfg) = self.provider_config(provider) else {
      return "none";
    };
    if cfg.kind == ANTIGRAVITY {
      return if antigravity_signed_in(data) {
        "subscription"
      } else {
        "none"
      };
    }
    if let Some(service) = subscription_service(provider, &cfg)
      && matches!(crate::auth::signed_in(data, service).await, Ok(true))
    {
      return "subscription";
    }
    if keys::get(data, provider, &cfg.kind).is_some() {
      "API key"
    } else {
      "none"
    }
  }

  /// The provider and model for `model` (`provider/model`, or the default).
  pub fn resolve(&self, data: &Path, model: Option<&str>) -> Result<(Provider, String, String)> {
    let full = model
      .or(self.default_model.as_deref())
      .unwrap_or(DEFAULT_MODEL);
    // Antigravity's bridge belongs to the data directory, and needs no key.
    if let Some((name, model)) = full.split_once('/')
      && let Some(cfg) = self.provider_config(name)
      && cfg.kind == ANTIGRAVITY
    {
      return Ok((
        antigravity_provider(name, data, cfg.effort)?,
        name.to_owned(),
        model.to_owned(),
      ));
    }
    self.resolve_using(model, |name, kind| keys::get(data, name, kind))
  }

  fn resolve_using(
    &self,
    model: Option<&str>,
    key: impl FnOnce(&str, &str) -> Option<String>,
  ) -> Result<(Provider, String, String)> {
    let full = model
      .or(self.default_model.as_deref())
      .unwrap_or(DEFAULT_MODEL);
    let (name, model) = full
      .split_once('/')
      .ok_or_else(|| Error::Config(format!("`{full}`: name a model as provider/model")))?;
    let cfg = self
      .provider_config(name)
      .ok_or_else(|| Error::Config(format!("no provider `{name}` in config.toml")))?;
    let key = key(name, &cfg.kind);
    let provider = match cfg.kind.as_str() {
      "anthropic" => {
        let mut a = match cfg.base_url {
          // Kimi: none of Claude's own fields.
          Some(u) if stitch_providers::is_moonshot(&u) => {
            Anthropic::moonshot(key.unwrap_or_default(), u)
          }
          Some(u) => Anthropic {
            base_url: u,
            ..Anthropic::new(key.unwrap_or_default())
          },
          None => Anthropic::new(key.unwrap_or_default()),
        };
        match cfg.effort.as_deref() {
          Some("") => a.effort = None,
          Some(e) => a.effort = Some(e.to_owned()),
          None => {}
        }
        a.thinking = cfg.thinking.unwrap_or(a.thinking);
        a.fallbacks = cfg.fallbacks.unwrap_or(a.fallbacks);
        a.cache = cfg.cache.unwrap_or(a.cache);
        a.cache_ttl = match cfg.cache_ttl.as_deref() {
          None | Some("5m") => None,
          Some("1h") => Some("1h".into()),
          Some(other) => {
            return Err(Error::Config(format!(
              "provider `{name}`: cache_ttl is `5m` or `1h`, not `{other}`"
            )));
          }
        };
        Provider::Anthropic(a)
      }
      "responses" => {
        let base = cfg
          .base_url
          .ok_or_else(|| Error::Config(format!("provider `{name}`: responses needs base_url")))?;
        let mut p = Responses::new(base, key.unwrap_or_default());
        p.effort = cfg.effort;
        p.cache_ttl = openai_cache_ttl(name, cfg.cache_ttl)?;
        Provider::Responses(p)
      }
      "openai" => {
        let base = cfg.base_url.ok_or_else(|| {
          Error::Config(format!(
            "provider `{name}`: an openai provider needs base_url"
          ))
        })?;
        let mut p = OpenAiCompat::new(base, key);
        p.cache_ttl = if p.openrouter {
          // Its breakpoints are Claude's: five minutes or an hour.
          match cfg.cache_ttl.as_deref() {
            None | Some("5m") => None,
            Some("1h") => cfg.cache_ttl,
            Some(other) => {
              return Err(Error::Config(format!(
                "provider `{name}`: cache_ttl is `5m` or `1h` on OpenRouter, not `{other}`"
              )));
            }
          }
        } else {
          openai_cache_ttl(name, cfg.cache_ttl)?
        };
        p.effort = cfg.effort.filter(|e| !e.is_empty());
        p.thinking = cfg.thinking;
        p.cache = cfg.cache.unwrap_or(true);
        if p.openrouter {
          p.router = Some(name.to_owned());
        }
        Provider::OpenAi(p)
      }
      "gemini" => {
        let mut p = Gemini::new(key.unwrap_or_default());
        if let Some(u) = cfg.base_url {
          p.base_url = u;
        }
        p.effort = cfg.effort.filter(|e| !e.is_empty());
        Provider::Gemini(p)
      }
      other => {
        return Err(Error::Config(format!(
          "provider `{name}`: unknown kind `{other}`"
        )));
      }
    };
    Ok((provider, name.to_owned(), model.to_owned()))
  }
}

/// The kind (and built-in provider) on the Google Antigravity plan.
pub const ANTIGRAVITY: &str = "antigravity";

#[cfg(not(any(target_os = "ios", target_os = "visionos")))]
fn antigravity_provider(_name: &str, data: &Path, effort: Option<String>) -> Result<Provider> {
  Ok(crate::antigravity::provider(data, effort))
}

#[cfg(any(target_os = "ios", target_os = "visionos"))]
fn antigravity_provider(name: &str, _data: &Path, _effort: Option<String>) -> Result<Provider> {
  Err(Error::Config(format!(
    "provider `{name}`: Antigravity runs on a desktop node (it drives the `agy` CLI)"
  )))
}

#[cfg(not(any(target_os = "ios", target_os = "visionos")))]
fn antigravity_signed_in(data: &Path) -> bool {
  crate::antigravity::signed_in(data)
}

#[cfg(any(target_os = "ios", target_os = "visionos"))]
fn antigravity_signed_in(_data: &Path) -> bool {
  false
}

/// `cache_ttl` for OpenAI's API: `30m`, the one value it takes.
/// `config.toml` with `[shards]` (the language's name until 2026-10-17)
/// as `[otter]`, or `None` when it has none (or does not parse: loading
/// says why). Its table headers are renamed in place, so the rest of the
/// file stays as it is; when that does not give the same settings (one
/// written otherwise, or `[otter]` there too), the two are merged and the
/// file written again from its values.
fn convert_shards(text: &str) -> Option<String> {
  let mut table: toml::Table = toml::from_str(text).ok()?;
  let shards = table.remove("shards")?;
  let renamed: String = text
    .split_inclusive('\n')
    .map(|line| {
      let indent = line.len() - line.trim_start().len();
      let rest = &line[indent..];
      match rest.strip_prefix("[shards") {
        Some(after) if after.starts_with(']') || after.starts_with('.') => {
          format!("{}[otter{after}", &line[..indent])
        }
        _ => line.to_owned(),
      }
    })
    .collect();
  let mut expected = table.clone();
  if !expected.contains_key("otter") {
    expected.insert("otter".into(), shards.clone());
    if toml::from_str::<toml::Table>(&renamed).ok() == Some(expected) {
      return Some(renamed);
    }
  }
  let otter = table
    .entry("otter")
    .or_insert_with(|| toml::Value::Table(Default::default()));
  merge_settings(otter, shards);
  toml::to_string(&table).ok()
}

/// Adds `from` to `into`: lists joined (without repeats), tables merged,
/// and `into`'s other values kept.
fn merge_settings(into: &mut toml::Value, from: toml::Value) {
  match (into, from) {
    (toml::Value::Table(into), toml::Value::Table(from)) => {
      for (k, v) in from {
        match into.get_mut(&k) {
          Some(mine) => merge_settings(mine, v),
          None => {
            into.insert(k, v);
          }
        }
      }
    }
    (toml::Value::Array(into), toml::Value::Array(from)) => {
      for v in from {
        if !into.contains(&v) {
          into.push(v);
        }
      }
    }
    _ => {}
  }
}

fn openai_cache_ttl(name: &str, ttl: Option<String>) -> Result<Option<String>> {
  match ttl.as_deref() {
    None => Ok(None),
    Some("30m") => Ok(ttl),
    Some(other) => Err(Error::Config(format!(
      "provider `{name}`: cache_ttl is `30m` for OpenAI's API, not `{other}`"
    ))),
  }
}

fn subscription_service(
  name: &str,
  cfg: &ProviderConfig,
) -> Option<stitch_providers::oauth::Service> {
  use stitch_providers::oauth::Service;
  match name {
    "anthropic"
      if cfg.kind == "anthropic"
        && cfg
          .base_url
          .as_deref()
          .is_none_or(|u| u.trim_end_matches('/') == "https://api.anthropic.com") =>
    {
      Some(Service::Anthropic)
    }
    "openai"
      if matches!(cfg.kind.as_str(), "openai" | "responses")
        && cfg
          .base_url
          .as_deref()
          .is_some_and(|u| u.trim_end_matches('/') == "https://api.openai.com/v1") =>
    {
      Some(Service::OpenAi)
    }
    "grok"
      if matches!(cfg.kind.as_str(), "openai" | "responses")
        && cfg
          .base_url
          .as_deref()
          .is_some_and(|u| u.trim_end_matches('/') == "https://api.x.ai/v1") =>
    {
      Some(Service::Grok)
    }
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn unknown_keys_are_warned_about() {
    let w = unknown_key_warnings("[shell]\ndefault_model = \"x\"\n");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("shell.default_model"), "{w:?}");
    assert!(w[0].contains("top level"), "{w:?}");
    let w = unknown_key_warnings("bogus = 1\n");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
      w[0].contains("bogus") && !w[0].contains("top level"),
      "{w:?}"
    );
    let clean = "default_model = \"x\"\nquick_model = \"y\"\n[shell]\n";
    assert!(unknown_key_warnings(clean).is_empty());
  }

  #[test]
  fn antigravity_is_built_in_on_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Config::default();
    c.providers.insert(
      "antigravity".into(),
      ProviderConfig {
        kind: "antigravity".into(),
        effort: Some("max".into()),
        ..ProviderConfig::default()
      },
    );
    let (p, name, model) = c
      .resolve(dir.path(), Some("antigravity/gemini-3.8-flash"))
      .unwrap();
    assert_eq!(
      (p.kind(), name.as_str(), model.as_str()),
      ("antigravity", "antigravity", "gemini-3.8-flash")
    );
    assert!(p.is_subscription() && !p.metered());
    assert!(p.label().starts_with("Google (Antigravity plan)"));
    assert!(Config::default().provider_config("antigravity").is_some());
    assert_eq!(
      crate::pricing::priced("antigravity/claude-sonnet-4-6", &Default::default(), None),
      None
    );
  }

  #[test]
  fn models_resolve_to_providers() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Config::default();
    c.providers.insert(
      "ollama".into(),
      ProviderConfig {
        kind: "openai".into(),
        base_url: Some("http://localhost:11434/v1".into()),
        ..ProviderConfig::default()
      },
    );
    c.save(dir.path()).unwrap();
    let c = Config::load(dir.path()).unwrap();
    let (p, name, model) = c.resolve_using(None, |_, _| None).unwrap();
    assert_eq!(
      (p.kind(), name.as_str(), model.as_str()),
      ("anthropic", "anthropic", "claude-opus-5-5")
    );
    let (p, _, model) = c
      .resolve_using(Some("ollama/qwen3:32b"), |_, _| None)
      .unwrap();
    assert_eq!((p.kind(), model.as_str()), ("openai", "qwen3:32b"));
    assert!(c.resolve_using(Some("nope/x"), |_, _| None).is_err());
    assert!(c.resolve_using(Some("noslash"), |_, _| None).is_err());
  }

  #[test]
  fn kimi_and_deepseek_are_built_in() {
    let c = Config::default();
    let (p, _, model) = c
      .resolve_using(Some("kimi/kimi-k3.1-pro"), |_, _| Some("sk-moon".into()))
      .unwrap();
    assert_eq!(model, "kimi-k3.1-pro");
    match &p {
      Provider::Anthropic(a) => {
        assert!(!a.claude && !a.thinking && !a.fallbacks);
        assert_eq!(a.base_url, "https://api.moonshot.ai/anthropic");
      }
      other => panic!("{other:?}"),
    }
    assert_eq!(p.label(), "Moonshot (API key)");
    // China's endpoint, set by hand, is Moonshot's too.
    let mut china = Config::default();
    china.providers.insert(
      "kimi".into(),
      ProviderConfig {
        kind: "anthropic".into(),
        base_url: Some("https://api.moonshot.cn/anthropic".into()),
        effort: Some("max".into()),
        ..ProviderConfig::default()
      },
    );
    match china
      .resolve_using(Some("kimi/kimi-k3"), |_, _| None)
      .unwrap()
      .0
    {
      Provider::Anthropic(a) => assert!(!a.claude && a.effort.as_deref() == Some("max")),
      other => panic!("{other:?}"),
    }
    // A proxy of Claude's API stays Claude's.
    let mut proxy = Config::default();
    proxy.providers.insert(
      "proxy".into(),
      ProviderConfig {
        kind: "anthropic".into(),
        base_url: Some("http://172.17.0.1:7178".into()),
        ..ProviderConfig::default()
      },
    );
    match proxy
      .resolve_using(Some("proxy/claude-opus-5-5"), |_, _| None)
      .unwrap()
      .0
    {
      Provider::Anthropic(a) => assert!(a.claude && a.thinking),
      other => panic!("{other:?}"),
    }
    let (p, _, _) = c
      .resolve_using(Some("deepseek/deepseek-flash"), |_, _| Some("sk-ds".into()))
      .unwrap();
    match &p {
      Provider::OpenAi(o) => {
        assert!(o.reasoning_back);
        assert_eq!(o.base_url, "https://api.deepseek.com");
      }
      other => panic!("{other:?}"),
    }
    assert_eq!(p.label(), "DeepSeek (API key)");
    // None inherits a subscription.
    for n in ["kimi", "deepseek", "openrouter"] {
      assert_eq!(
        subscription_service(n, &c.provider_config(n).unwrap()),
        None
      );
    }
  }

  #[test]
  fn openrouter_is_built_in_and_takes_any_model_id() {
    let c = Config::default();
    // The model is everything after the provider, slashes and all.
    let (p, name, model) = c
      .resolve_using(
        Some("openrouter/~anthropic/claude-sonnet-latest"),
        |n, _| (n == "openrouter").then(|| "sk-or-v1-x".into()),
      )
      .unwrap();
    assert_eq!(
      (name.as_str(), model.as_str()),
      ("openrouter", "~anthropic/claude-sonnet-latest")
    );
    match &p {
      Provider::OpenAi(o) => {
        assert!(o.openrouter && o.cache && !o.reasoning_back);
        assert_eq!(o.base_url, "https://openrouter.ai/api/v1");
        assert_eq!((o.thinking, o.cache_ttl.as_deref()), (None, None));
        // Its responses' models are named with it.
        assert_eq!(o.router.as_deref(), Some("openrouter"));
      }
      other => panic!("{other:?}"),
    }
    assert_eq!(p.label(), "OpenRouter (API key)");
    // Its settings: reasoning, Claude's cache lifetimes.
    let mut set = Config::default();
    set.providers.insert(
      "openrouter".into(),
      ProviderConfig {
        kind: "openai".into(),
        base_url: Some(stitch_providers::OPENROUTER_URL.into()),
        effort: Some("high".into()),
        thinking: Some(false),
        cache: Some(false),
        cache_ttl: Some("1h".into()),
        ..ProviderConfig::default()
      },
    );
    match set
      .resolve_using(Some("openrouter/openai/gpt-5.6-sol"), |_, _| None)
      .unwrap()
      .0
    {
      Provider::OpenAi(o) => {
        assert_eq!(o.effort.as_deref(), Some("high"));
        assert_eq!(o.thinking, Some(false));
        assert!(!o.cache);
        assert_eq!(o.cache_ttl.as_deref(), Some("1h"));
      }
      other => panic!("{other:?}"),
    }
    set.providers.get_mut("openrouter").unwrap().cache_ttl = Some("30m".into());
    assert!(
      set
        .resolve_using(Some("openrouter/x/y"), |_, _| None)
        .is_err()
    );
  }

  #[test]
  fn cache_lifetimes_are_the_providers_own() {
    let with = |kind: &str, ttl: &str| {
      let mut c = Config::default();
      c.providers.insert(
        "p".into(),
        ProviderConfig {
          kind: kind.into(),
          base_url: Some("https://api.openai.com/v1".into()),
          cache_ttl: Some(ttl.into()),
          ..ProviderConfig::default()
        },
      );
      c.resolve_using(Some("p/m"), |_, _| None).map(|(p, _, _)| p)
    };
    match with("anthropic", "1h").unwrap() {
      Provider::Anthropic(a) => assert_eq!(a.cache_ttl.as_deref(), Some("1h")),
      other => panic!("{other:?}"),
    }
    match with("anthropic", "5m").unwrap() {
      Provider::Anthropic(a) => assert_eq!(a.cache_ttl, None),
      other => panic!("{other:?}"),
    }
    match with("responses", "30m").unwrap() {
      Provider::Responses(r) => assert_eq!(r.cache_ttl.as_deref(), Some("30m")),
      other => panic!("{other:?}"),
    }
    assert!(with("anthropic", "30m").is_err());
    assert!(with("responses", "1h").is_err());
    assert!(with("openai", "24h").is_err());
  }
}

#[cfg(test)]
mod subscription_tests {
  use super::*;
  use stitch_providers::oauth::Service;
  #[test]
  fn sessions_are_scoped_to_named_official_endpoints() {
    let c = Config::default();
    for (name, service, url) in [
      ("anthropic", Service::Anthropic, "https://api.anthropic.com"),
      ("openai", Service::OpenAi, "https://api.openai.com/v1"),
      ("grok", Service::Grok, "https://api.x.ai/v1"),
    ] {
      let mut cfg = c.provider_config(name).unwrap();
      assert_eq!(subscription_service(name, &cfg), Some(service));
      assert_eq!(subscription_service("custom", &cfg), None);
      for foreign in [
        "http://localhost:8080",
        "https://api.openai.com.evil/v1",
        "https://api.x.ai/v1",
        "https://api.anthropic.com",
      ] {
        if foreign == url {
          continue;
        }
        cfg.base_url = Some(foreign.into());
        assert_eq!(subscription_service(name, &cfg), None);
      }
      cfg.base_url = Some(format!("{url}/"));
      assert_eq!(subscription_service(name, &cfg), Some(service));
      cfg.kind = "unrelated".into();
      assert_eq!(subscription_service(name, &cfg), None);
    }
  }

  #[test]
  fn shards_settings_become_otter_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    // Renamed in place: comments and the rest kept.
    let before = r#"node = "mbp" # this machine

# Tools that run without asking.
[shards]
allow = ["google_flights_*"]
timeout_secs = 60

  [shards.places."global"]
allow = ["echo"]

[mcp]
enable = ["github"]
"#;
    std::fs::write(&path, before).unwrap();
    let c = Config::load(dir.path()).unwrap();
    let otter = c.otter.unwrap();
    assert_eq!(otter.allow, ["google_flights_*"]);
    assert_eq!(otter.timeout_secs, Some(60));
    assert_eq!(otter.places["global"].allow, ["echo"]);
    assert_eq!(
      std::fs::read_to_string(&path).unwrap(),
      before
        .replace("[shards]", "[otter]")
        .replace("[shards.places", "[otter.places")
    );
    // Once: the file is left alone after.
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    Config::load(dir.path()).unwrap();
    assert_eq!(
      std::fs::metadata(&path).unwrap().modified().unwrap(),
      modified
    );

    // Written otherwise, and next to an `[otter]`: merged.
    std::fs::write(
      &path,
      r#"shards = { allow = ["a", "b"], timeout_secs = 5, places = { global = { allow = ["c"] } } }
[otter]
allow = ["b", "d"]
timeout_secs = 9
"#,
    )
    .unwrap();
    let otter = Config::load(dir.path()).unwrap().otter.unwrap();
    assert_eq!(otter.allow, ["b", "d", "a"]);
    assert_eq!(otter.timeout_secs, Some(9));
    assert_eq!(otter.places["global"].allow, ["c"]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("shards"), "{text}");

    // Nothing to convert, or not TOML: untouched.
    assert_eq!(convert_shards("[otter]\nallow = []\n"), None);
    assert_eq!(convert_shards("[shards"), None);
  }
}
