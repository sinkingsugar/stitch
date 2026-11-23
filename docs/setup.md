# Setting Stitch up

A manual for an agent (Stitch itself, Claude Code) that sets Stitch up on the
user's behalf, and for the user. Every command, file and key here is checked
against the code (`crates/stitch-cli/src/main.rs`,
`crates/stitch-node/src/{config,mcp,toolset,placeset,keys,tools}.rs`); the
design is in [design.md](design.md). Each step says how to verify it.
The [command overview](commands.md), [full CLI help](cli.md),
[TUI guide](tui.md) and [feature guide](features.md) cover the wider surface.

## Contents

- [1. Install, update, restart](#1-install-update-restart)
- [2. Pairing, peers, sync](#2-pairing-peers-sync)
- [3. Providers, model choices, secrets and variables](#3-providers)
- [4. Shell approvals](#4-shell-approvals)
- [5. MCP servers](#5-mcp-servers)
- [5a. Otter tools](#5a-tools-written-in-otter)
- [6. Per-thread tool and skill sets](#6-tool-and-skill-sets-per-thread)
- [7. Places and folder-file sync](#7-places-settings-and-skills-of-a-folder-or-checkout)
- [8. Shared Mail.app tools example](#8-worked-example-mail-with-mailapp-on-the-mac-and-from-other-nodes)
- [9. Detached threads and budgets](#9-detached-threads-and-budgets)
- [10. Memory](#10-memory)
- [10a. Your instructions](#10a-your-instructions)
- [10b. Thread management](#10b-threads-search-titles-pins-forks-summaries-and-compaction)
- [10c. Runtime limits, scheduling and notices](#10c-runtime-limits-scheduling-and-notices)
- [11. Troubleshooting](#11-troubleshooting)

Conventions:

- `<data>` is the node's data directory: `~/.stitch`, or `--data <dir>` /
  `STITCH_DATA`. Everything below is relative to it unless the path is absolute.
- `<data>/config.toml` is per node and never syncs. Secret values stay in
  credential storage (keychain or owner-only files), never plaintext records.
  New secrets sync sealed unless device-only; subscription sessions stay local
  (see §3).
- A node name is `[a-z0-9]{1,12}`, unique among the user's nodes (`laptop`,
  `nas`). MCP server names are `[a-z0-9-]`, at most 32 characters.
- When config is read:
  - `[mcp]` is read again when each turn starts, so changes apply from the
    next turn.
  - `stitch mode` (and `stitch yolo`) takes effect at once, and so do `always` answers, which
    the node adds to `[shell.places]` itself.
  - Everything else in `config.toml` (`[shell]`, `[detached]`,
    `[providers]`, `[memory]`, `[browser]`, `[threads]`, `default_model`,
    `quick_model`, `sync_interval_secs`) is read when the node starts. After changing it by
    hand, restart the node (below).

## 1. Install, update, restart

### Install

Building needs a Rust toolchain, CMake (Ninja is used when present), a C++
toolchain and the llama.cpp submodule. Otter (the language tools are
written in, and its runtime) and engram are git dependencies Cargo fetches
at the commits `Cargo.lock` pins (public: no checkout beside Stitch, no
credentials):

```sh
git clone <stitch repo> ~/devel/stitch && cd ~/devel/stitch
git submodule update --init                                    # llama.cpp
cargo install --locked --path crates/stitch-cli
```

For work on Otter or engram together with Stitch, clone them beside it
(`../otter`, `../engram`) and `just dev-link` (AGENTS.md); `just
dev-unlink` goes back to the pins.

`cargo install --locked --path crates/stitch-cli` installs two binaries into
`~/.cargo/bin` (or `$CARGO_HOME/bin`): `stitch` (the node, CLI and TUI) and
`stitch-shell-helper`. On macOS prefer `just install`: it does the same, then
signs `stitch` with your Apple Development identity (or
`STITCH_SIGN_IDENTITY`). Unsigned, the binary is ad-hoc signed by its hash,
so the keychain takes each rebuild for a new app and asks again for every
key; signed, "Always Allow" holds across rebuilds.

Verify:

```sh
command -v stitch                                   # ~/.cargo/bin/stitch
ls -l "$(dirname "$(command -v stitch)")"/stitch-shell-helper
stitch --help
```

**Why the helper must sit next to `stitch` (macOS).** On macOS the node does
not start shells itself. `stitch-shell-helper` is a small single-threaded
process without the node's frameworks; it forks each shell (`setsid`,
`TIOCSCTTY`, exec) and reports the shell's pid and how it ended (design §2.3).
The node finds the helper like this:

1. The path in `STITCH_SHELL_HELPER`, read from the node's own environment.
   Use it for a build of your own or a test.
2. Otherwise, `stitch-shell-helper` in the same directory as the running
   `stitch` executable.

If the helper is missing, the node logs this once:
`shells: <path> is missing (…); shells start through /bin/sh, …`. Shells then
start through a `/bin/sh` trampoline. They still work, but the helper is the
supported path.

Node and helper check each other's protocol version. If they differ, no shell
starts and the error says how to fix it (see §11). So:

- Never copy `stitch` somewhere without its helper.
- Always install the two together.

Linux does not use the helper.

### First setup of a node

```sh
stitch setup --name laptop          # writes node = "laptop" to <data>/config.toml,
                                 # creates <data>/node.key and the home workspace <data>/home
```

Options:

- `--home <dir>`: put the home workspace somewhere else.
- `--no-home`: create no home workspace. Use this for a node that will
  `stitch join` another (§2).
- `--local`: direct addresses only (`discovery = "local"`: no relays, nothing
  published to discovery).

Verify: `stitch id` prints the endpoint ID. Running `stitch` alone opens the
TUI.

### The node process

Any command that needs the node starts it in the background when none runs:
`stitch --data <data> node`, its output appended to `<data>/node.log`. It keeps
running after that command exits.

- **It inherits the environment of the command that started it:** `PATH`,
  `HOME`, `STITCH_SHELL_HELPER`, `STITCH_*_API_KEY`. MCP servers and detached
  threads see the node's environment (a chat's shells get the chatting
  client's). So start the node from a shell whose `PATH` has what servers need
  (Homebrew's `/opt/homebrew/bin`, `~/.cargo/bin`, …), or give full paths in
  the configuration (§5).
- `stitch node` runs it in the foreground (logs to the terminal). Use it under
  launchd or systemd, or to watch it.
- `stitch node status` prints `node <name> (<endpoint>)`, `build: …` (the
  commit of Stitch it was built from, `-dirty` for uncommitted changes, and
  Otter's and engram's as its `Cargo.lock` pins them, or "(linked)" with
  the commit of a `just dev-link`ed checkout; `stitch --version` says the same of a binary), a
  restart it waits to make, `repositories: …` and `peers: …`. It prints
  `no node runs` and does not start one.
- `stitch node stop` stops it, with its shell sessions and the MCP servers it
  started; `--when-idle` waits until nothing runs.
- `stitch node restart` restarts it once nothing runs (`--now`: at once).
  The node starts its successor itself (under a service, it exits for the
  service to), so a restart asked from an agent's sandboxed shell never runs
  the new node inside that sandbox or with that shell's environment.

**Restart** (needed after changing `config.toml` outside `[mcp]`, after
`stitch memory install` (done for you); an update restarts by itself):

```sh
stitch jobs            # what runs now: shell jobs and detached threads
stitch node restart    # once idle; --now to cut what runs
stitch node status     # build: …, or "stopping for … once idle (now: …)"
```

Stopping ends every shell session and job. Detached threads the node started
resume when it starts again. A detached thread waiting on an approval keeps
its question open, and resumes on the answer, even one given while the node
was down. A chat's question is withdrawn once the node is back, since chats
do not resume.

**A new build restarts the node.** The node watches its own binary (the
path it was started from, every 10 s): once another build is installed
there (`just install`, `cargo install`; the binary and its helper unchanged
over two looks, so never half-written) whose contents differ from the
running one's (a build is known by the hash of its binary; the commits
`--version` says are for people), and it runs, the node restarts into it
once idle, never cutting what runs. A newer client (another protocol) is
handed over to the same way when such a build is installed; otherwise a node
outside a service stops once idle for the client to start its own, and one
under a service stays (the service starts only its own binary). A binary
that does not run is left alone (the log says so). While a node restarts,
`<data>/restarting` tells clients to wait for its successor rather than
start a node of their own (60 s at most).

### A node as a service

```sh
just install                                  # the binary the service runs
stitch node service install                   # systemd user unit / LaunchAgent
stitch node service install --update          # and a self-update every 30 min (~/devel/stitch)
stitch node service install --print           # only show the files
stitch node service uninstall
```

- **Linux:** `~/.config/systemd/user/stitch.service` (another data
  directory: `stitch-<path>.service`), enabled and started; its output goes
  to `<data>/node.log` as before. Without lingering (`sudo loginctl
  enable-linger $USER`) it runs only while you are logged in; `install` says
  so.
- **macOS:** `~/Library/LaunchAgents/dev.stitch.service.<path>.plist` in the
  GUI session (the keychain answers there), `KeepAlive` on failure only. Its
  label is its own (not that of the job the browser host starts a node in),
  and installing never boots out a loaded agent whose node runs: launchd
  keeps the loaded definition until the agent loads again (at login, or
  `install` again while no node runs); `install` says so.
- The unit gets the `PATH` and `SHELL` of the shell you ran `install` from:
  run it from a login shell.
- It runs `stitch node` with `STITCH_SUPERVISED=1`: a restart the node asks
  for (a new build installed, `stitch node restart`) is an exit with status
  75, which the service answers by starting it again; `stitch node stop`
  stops it until the next client or login. Whether a node is the service's
  is decided once at its start (on Linux, the unit's `MainPID` is the node:
  so mini's unit from before `STITCH_SUPERVISED` counts too; on macOS, its
  launchd label), and the variables are then removed from its environment,
  so its shells and a node one of them starts never take it for theirs.
- A node already running outside the service is asked to stop once idle; the
  service's node waits for the lock and takes over then (nothing is cut).
  The service's node asks so itself too when it starts and finds one (at
  login, after a reboot). A node already running under the unit being
  installed restarts into the new unit once idle. On macOS, an agent an
  earlier `install` wrote as `dev.stitch.node.<path>.plist` is removed (and
  booted out unless its node runs: that one stops once idle).
- **mini, once:** its unit predates `stitch node service install` (no
  `KillMode=process`, so a restart takes the agents' own processes down
  with it, and no `<data>/service`, so clients do not start it). Run
  `stitch node service install --update` there once, from a login shell
  after the update that brings this build: it rewrites the unit and the
  timer, restarts the node into the new unit once idle, and writes
  `<data>/service`.
- `<data>/service` names the service, written last, once the service is in
  place: a client that finds no node starts the service (`systemctl --user
  start`, `launchctl kickstart`) instead of a node of its own; when that
  fails (no user bus over SSH, before the GUI login), it starts a node as
  before. A node outside the service that is to restart starts the service
  instead of a successor of its own.
- `--update[=<checkout>]` adds `stitch-update.timer` (systemd) or
  `dev.stitch.update` (launchd, logging to `<data>/update.log`), running
  `~/.local/bin/stitch-self-update` (`deploy/node/self-update.sh`): only
  when Stitch is a clean checkout on `main` (and not `just dev-link`ed), it
  pulls it, and when the installed build (`stitch --version`) is not its
  label (so a build that failed is tried again), builds and installs
  (`deploy/node/install.sh`, as `just install`: `--locked`, staged, signed
  on macOS before it is put in place, without ever asking: a keychain that
  would ask leaves it ad-hoc); the node restarts into the new build once
  idle.

### Builds kept, health and rollback

Each build a node runs is kept in `<data>/bin/<commit>-<hash>/` (the hash
of the binary's contents: two builds of one commit, an Otter-only change,
are two builds), with its shell helper, the build, its hash and the store
format it needs: the healthy one, the one before it, the one on trial and
two more. The copy is of the running image, synced and checked against its
hash; a kept build that no longer matches its hash is never installed.
Binaries under a `target/` directory (`cargo run`, tests) are not kept. A
build new to the node is **on trial** until it has run 60 s and answers on
its socket; then it is the healthy one (`node.log`: `upgrade: build …
healthy`). `<data>/bin/state.json` says which (one that cannot be read is
kept as `state.json.bad` and said).

- A build on trial that does not answer on its socket 2 minutes after
  that gives up: it exits with status 70, an error. The watchdog runs from
  before the node's start, so a build that hangs while holding the lock (its
  store, a migration, its socket) gives up too.
- A rollback is only to a build kept whole (its hash checked); when the
  kept copy is damaged or missing, the new build keeps running and the log
  says why, rather than failing and starting again for ever.
- SIGTERM while a node starts, before it handles the signal itself, is a
  stop (`<data>/bin/early-stop`), not a failed start.
- Under a service, a build on trial that starts 3 times without getting
  healthy gives way to the healthy build: it is installed back where the
  binary is (with its helper) and the service starts it. Starts that ended
  because a stop or restart was asked for do not count.
- Without a service, the process that restarted the node watches its
  successor (as long as it holds the lock and is not healthy yet): one that
  ends with an error or a crash before it is healthy, or that never gets as
  far as taking the lock, gives way to the healthy build the same way. One that ends because it was asked to stop (status 0,
  or its `<data>/stopping` marker) is let be.
- A build that failed is not restarted into again when it is installed again,
  and gives way at once when something starts it anyway (`failed` in
  `state.json`; remove it there to try again).
- What happened is in `node.log` and sent as a notice to your phones.
- Never across a store format (`STORE_FORMAT`; a test fails when the code of
  the store's files changes, so the format is decided with it): the node
  keeps trying the new build and says it needs you.
- A stop or restart someone asked for that waits for the node to be idle
  waits at most 30 minutes (`STITCH_DRAIN_SECS`), then stops what runs;
  detached threads resume where they were. A restart nobody asked for (a
  new build installed, a newer client) waits for idle however long it takes.
- Not covered: under a service, a build that fails before its own start
  code runs (it cannot be loaded at all) is restarted by the service until
  its start limit, not rolled back.

`stitch node builds` says all this (`--healthy-for <secs>` succeeds only
when the build installed runs and has been healthy here that long).

### The rollout and Stitch fixing itself

New builds go canary first (docs/self-improve.md §4): each node's update
timer follows a branch of Stitch that the node before it promotes, the
same commit (its `Cargo.lock` pins Otter and engram), once that build's process ran healthy
there for 30 minutes without a stop.

```sh
# tower, the canary: follows main, promotes to fleet
stitch node service install --update --promote fleet
# mini: follows fleet, promotes to fleet-mac
stitch node service install --update --follow fleet --promote fleet-mac
# laptop: follows fleet-mac, then uploads the app to TestFlight (the phone),
# from the app's checkout (its own repository), built against this one
stitch node service install --update --follow fleet-mac --testflight ~/devel/app
```

- The checkout stays clean on `main`, exactly `origin/<branch>`: a main
  with commits of its own, or diverged, is not built (the update log says
  so: `~/.stitch/update.log` on macOS, `journalctl --user -u
  stitch-update` on Linux).
- Promoting pushes the branch in Stitch, from that node's checkout (push access needed: "Push credentials" below); `fleet`
  and `fleet-mac` appear with the first promotion, and a node following one
  that does not exist yet waits. A failed push is told to you.
- A build that failed is not tried again until a commit moves
  (`stitch-self-update --force` to retry); you are told once.
- What lands on main is installed: a fixer's change to a protected path
  was approved by you when it landed ("Stitch fixing itself" below).
- `--testflight <app checkout>` runs that checkout's justfile by path (`just -f
  <checkout>/justfile ios device`, then `… testflight`, with `STITCH_DIR` the
  Stitch checkout); the app checkout is held to the same rules as Stitch's
  (clean, on main, exactly its origin's `main`, or `STITCH_APP_FOLLOW`) and
  promoted with it. A plain `--testflight` uses the Stitch checkout's own
  justfile when it holds the app; otherwise nothing is uploaded, and the
  update log says so.
  It tries at most 3 times a pair of commits (Stitch's and the app's), an
  hour apart, then tells you.

### Push credentials

Promotion pushes `fleet` and `fleet-mac` from the update unit with the
node's own git credentials (an SSH key or credential helper the unit can
use without a prompt: `GIT_TERMINAL_PROMPT=0`, no agent). Any agent
running as that user could use them too; that risk is accepted
(docs/self-improve.md §5).

### Stitch fixing itself

Harness issues (bugs in Stitch itself) are filed by agents with
`report_issue`, or by you: `stitch issue report "<title>" --what "…"`.
`stitch issue list`, `show <id>`, `set <id> <state> [--note …] [--fix
<commit>]`. Self-improvement is off until you turn it on, with settings
that sync to every node:

```sh
stitch issue settings --node tower --budget 5 --daily 20 --on
stitch issue settings                  # what stands, and today's spending
stitch issue settings --off
```

(also `--place <checkout>`, `--every <cron>`, `--model <provider/model>`;
`[self_improve]` in a node's config.toml is only its fallback). The fixer
node must be the canary (its update timer follows main). A fixer is
started by the `self_improve` tool (off for every thread unless turned on
by its name, `stitch thread tools <thread> +self_improve`; it asks you unless a chat of
yours or the harness-issues schedule calls it) or by `stitch issue fix
<id>` from your terminal. It runs on the fixer node, in its checkout, with
the `stitch-dev` skill; one at a time, within the day's cap (UTC), two
attempts per issue before you are asked, and only once the last fix runs
in the canary's build and the canary has been up with it for 30 minutes.

**Who asks is the process, not what it says.** The node tells you from a
thread by the connecting process (walked up to a shell session it
started), never by `STITCH_THREAD`: `stitch issue settings|set|fix|schedule`
from a thread's shell ask you first, whatever yolo says.

**A fixer has none of your powers**, whatever the node's approval mode:
it runs sandboxed (writes in its worktree), never yolo, and asks you for
any push but its own `fix-*` branches, `gh` beyond reading, `stitch issue
settings|set|fix`, the self-update, services, tool sets, keys and secrets.
It lands through the node: `stitch issue land <id> <commit> --summary
"…"` fast-forwards main to a commit CI passed, and tells you. When the
commit touches a protected path (`tools/protected-paths`: the approval
rules and fixer mode, who asks the node, the sandbox, self-improvement,
`deploy/`, `.github/`), it first asks you: a notification with the TL;DR,
and in the approvals (app, `stitch attention`, the TUI's `v`) the protected
files' diff in full as the node read it, a stat of the rest, and the
fixer's summary marked as its words. Only you can answer it: an answer
from a thread's commands, or from any sandboxed process, is refused.
Declined, the issue is open again with your reason and the branch stays.

On the fixer node, `stitch issue schedule` makes the harness-issues
schedule: at each `every`, its check (the node's own, no model) wakes a
thread only when open issues changed, which triages them and calls
`self_improve`.

### Update a node

```sh
cd ~/devel/stitch
just update          # or, by hand:
git pull
git submodule update --init
cargo install --locked --path crates/stitch-cli
stitch jobs
```

The `justfile` has the chores: `just update`, `just update-remote <host>`
(over SSH), `just restart`, `just start`, `just log`, `just check` (on macOS
with `just check-apple`: the crates built for iOS and visionOS). A node of a
new version migrates its store on its first start when it has to, and says so
in `node.log`.

The running node restarts into the installed build once idle (above). When
a newer `stitch` (a newer local protocol) finds an older node:

- If the old node is idle, it stops and the newer `stitch` starts its own node.
- If it is busy (a chat or thread at work, a command running in a shell), it
  stops once idle.

`stitch node stop` forces the change. Verify that `node.log` shows a fresh
start (`tail -n 20 ~/.stitch/node.log`) and that `stitch node status` answers.

### Update another node

Over SSH (or at its terminal), on that machine, run the same steps:
`git pull`, `git submodule update --init`,
`cargo install --locked --path crates/stitch-cli`, then restart as above. Start the
node from a login shell (`ssh host -t 'zsh -lc "stitch jobs"'`) so it gets a
full `PATH`; `just update-remote <host>` does all of it.

Nodes whose protocol versions differ do not sync at all (version 2 since
segments are one per turn): update every node to the same commit, e.g.
`just update-remote <host>`. Within one protocol version, a newer node answers
an older one's question it does not know with "a question this node does not
know".

## 2. Pairing, peers, sync

On a node that is already set up:

```sh
stitch invite                 # QR code and ticket; valid once, for 10 minutes (--minutes N)
```

On the new machine, after installing:

```sh
stitch setup --name nas --no-home
stitch join <ticket>          # --home <dir> to hold the home workspace elsewhere
```

`join` prints:

- `joined <inviter> (<id>)`;
- `also trusting <name>` for each node of the fleet;
- the first sync of the home workspace:
  `<repo>: N in, M out, blobs a in, b out`.

It stops a node that was running there, which starts again with the next
command. A ticket is withdrawn after 5 wrong secrets.

Verify on both machines:

```sh
stitch peer ls                # name, endpoint ID, addresses
stitch sync                   # "<peer> <repo>: …" per peer and repository
stitch node status            # peers: …
```

Other peer commands:

- `stitch peer add <name> <id> --addr <ip:port>`: trust by hand. Needed with
  `--local` discovery.
- `stitch peer remove <name>`: writes a `revoke` record, which spreads to
  every node.
- `stitch sync <peer>`: syncs with that node only.

Nodes sync on their own every `sync_interval_secs` (default 300, `0` off), and
when work needs it (an assignment, a report, an approval).

## 3. Providers

Subscriptions (the built-in `anthropic`, `openai`, `grok` at their official
endpoints only):

```sh
stitch auth login anthropic   # or openai, grok; --file: owner-only file instead of the keychain
stitch auth status            # one line per provider; never prints tokens
stitch auth logout openai
```

`login` prints a URL and waits up to 10 minutes for the callback on
localhost. On a remote machine, forward the callback port to the browser's
machine first: Anthropic 53692, OpenAI 1455, Grok 53694.

API keys:

```sh
stitch key set anthropic                          # prompts without echo on a terminal
printf '%s' "$KEY" | stitch key set anthropic     # an agent: from standard input
stitch key set brave                              # search_web (or perplexity)
stitch key rm anthropic
```

`set` says whether it kept the key in the keychain or in
`<data>/credentials.toml` (with `--file`, or when there is no keychain).
A provider's key is the secret of the provider's name: `stitch key set
anthropic` is `stitch secret set anthropic`.

Secrets and variables (design §2.19):

```sh
stitch secret set openai                      # global; prompts without echo
stitch secret set db-password --here          # the project or folder you are in
stitch secret set bank-pin --device-only      # never leaves this node
stitch secret ls                              # where each is held and used, no values
stitch secret get openai --reveal             # its value
stitch secret rm openai
stitch var set HF_HOME /data/hf               # shells get variables by name
stitch var ls
```

A shell gets a secret only where `[env]` maps it, in config.toml (every
shell on this node) or in a place's `.stitch/settings.toml`:

```toml
[env]
OPENAI_API_KEY = "secret:openai"   # the place's secret of that name, else the global one
REGION = "var:region"              # a variable
```

Tool output shows `«secret:<name>»` where a value was, before the model or
any record sees it. Keys stored before secrets existed are listed as this
device's only; `stitch secret set <name> --sync --keep-value` shares one.

The node reads a key from these places, first found wins:

1. `<data>/credentials.toml`.
2. `STITCH_<NAME>_API_KEY` in the node's environment (upper case, `-` → `_`).
3. The provider's usual variable: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
   `XAI_API_KEY`, `GEMINI_API_KEY` (else `GOOGLE_API_KEY`), `MOONSHOT_API_KEY`
   (else `KIMI_API_KEY`), `DEEPSEEK_API_KEY`, `OPENROUTER_API_KEY`, `BRAVE_API_KEY`,
   `PERPLEXITY_API_KEY`.
4. The keychain entry for this data directory.

Kimi (`kimi/<model>`, Moonshot's Anthropic-compatible endpoint) and DeepSeek
(`deepseek/<model>`, Chat Completions) are built in too: `stitch key set kimi`
or `stitch key set deepseek`, no `[providers]` entry needed. For Moonshot's
China endpoint, set `[providers.kimi] base_url = "https://api.moonshot.cn/anthropic"`.

OpenRouter is built in too (`stitch key set openrouter`, or
`OPENROUTER_API_KEY`): any model it routes is `openrouter/<its ID>`, listed
or not, e.g. `openrouter/anthropic/claude-sonnet-5.5`,
`openrouter/~openai/gpt-sol-latest` or `openrouter/deepseek/deepseek-v4-pro:nitro`
(`stitch model default openrouter/…`, `/model openrouter/…` in the TUI,
**Custom Model…** in the app's model menu and Settings). Tools, streaming,
reasoning (sent back with tool results, signatures and all), images, PDFs
and prompt caching (breakpoints on Claude, sticky routing per thread) work,
and each call's cost is what OpenRouter billed. Calls are attributed to
Stitch (`HTTP-Referer` its repository, title `Stitch`), a public app in
OpenRouter's rankings. Optional settings:

```toml
[providers.openrouter]
kind = "openai"
base_url = "https://openrouter.ai/api/v1"
# effort = "high"     # reasoning.effort for every model; unset: each model's default
# thinking = false    # reasoning on or off when no effort is set
# cache = false       # no cache breakpoints on Claude models
# cache_ttl = "1h"    # Claude's breakpoints: "5m" (default) or "1h"
```

`models` lists only the OpenRouter models in use (default, quick, those with
settings) and counts the rest; the TUI's `/model` and the app list them all.

Gemini (`stitch key set gemini`, or `GEMINI_API_KEY`) is built in, like
`openai` and `grok`: its models come from Google's API (`gemini/<model>`). It
reads audio and video attachments (large ones are uploaded to Google's Files
API, kept there 48 hours), and its image models answer with images, saved as
the reply's attachments. `effort` in `[providers.gemini]` sets the thinking
level (`minimal`, `low`, `medium`, `high`, as the model takes).

With a Gemini key every thread also has `generate_image` (any model calls it
to make or edit images; turn it off with `-images`, as other tool groups). Its
model, in `config.toml`:

```toml
[images]
model = "gemini/gemini-3-pro-image"   # default: gemini/gemini-3.1-flash-image
```

**Which wins, a key or a subscription.** A subscription you signed in to wins
over an API key for the same built-in provider, whether the key is stored or in
the environment (`Config::resolve_authenticated`). Keys serve providers without
a subscription, and custom endpoints never inherit subscription tokens. To use
API billing for a built-in provider, sign out (`stitch auth logout
<provider>`).

Use `stitch model default <provider/model>` and `stitch model quick <provider/model>`
for choices synced to every node; a project's `[models] default` and `quick`
in `.stitch/settings.toml` override them there. The terminal and phone show the
node's effective next-turn model. `/model default` (or Follow default on the
phone) follows that role rather than pinning today's model. A concrete per-thread
pick stays fixed. Existing phone Settings choices remain as device-only **App
override** settings; choose None to inherit the thread/synced default.

With nothing set, a node uses a built-in pair for the first provider it has
credentials for (signed-in subscriptions first, then API keys; anthropic,
openai, gemini, grok, kimi, deepseek, openrouter): anthropic `claude-opus-5-5` and
`claude-sonnet-5-5`, openai `gpt-5.6-sol` (quick is the same), gemini
`gemini-3.1-pro-preview` and `gemini-3.5-flash`, kimi `kimi-k3.1-pro` and
`kimi-k2.7-code`, deepseek `deepseek-v4-pro` and `deepseek-flash`, openrouter
`anthropic/claude-opus-5.5` and `anthropic/claude-sonnet-5.5`. Quick is the table's only when the
default is that provider's own (a Haiku default keeps Haiku as quick). A model
marked `--reserved` is skipped. `stitch model ls` marks them `(built-in)`;
signing in or out changes them at once. A node with no credentials says
`anthropic/claude-opus-5-5`.

Other providers and this node's own models, in `config.toml` (restart after).
Unset by default; when set, they win on this node over the synced ones
(`stitch model default|quick`), a project's `[models]` still winning in it:

```toml
default_model = "anthropic/claude-opus-5-5"   # this node's default
quick_model = "anthropic/claude-sonnet-5-5"   # this node's quick model

[providers.ollama]
kind = "openai"                  # Chat Completions; or "anthropic", "responses", "gemini"
base_url = "http://localhost:11434/v1"
# effort = "high"                # anthropic: low … max; "" leaves the model's default
# thinking = true                # anthropic
# fallbacks = true               # anthropic
# cache = true                   # anthropic
# cache_ttl = "1h"               # anthropic: "5m" (default) or "1h"; OpenAI's API: "30m"
# context_window = 200000        # tokens its models take in: compaction, §10b
```

A model is named `provider/model`. A custom endpoint never uses a
subscription.

To verify, use one of these:

- `stitch model ls` (`--node <name>` for another node's): the models, ranked
  as agents see them, with prices and your settings (`stitch model set`
  marks one reserved, or gives its price or a note).
- `stitch auth status` for subscriptions.
- In the TUI, `/model`, which asks each provider for its models.
- One cheap call:
  `stitch chat --global -m <provider/model> "Reply with OK"`.
  Its last line on stderr names the thread, the model, tokens and cost:
  `48313 in (40102 cached), 6120 out` is the whole prompt, the part read
  from the cache, then the output (`…, 512 to cache` when some was written
  to it).

**Prompt caching** is on. Anthropic caches for five minutes by default;
`cache_ttl = "1h"` keeps a thread's prompt cached for an hour, worth it when
its turns come more than five minutes apart (detached threads waiting on
reports, a chat you come back to): writes cost twice input instead of 1.25
times, reads the same. OpenAI and xAI cache unasked; a thread's requests
carry its ID so they reach the cache that holds its prompt.

Threads see what a node runs through the `models` tool.

## 4. Shell approvals

```toml
[shell]
approval = "sandbox"      # the default on macOS, and on Linux with Landlock:
                          # see below. "ask": what is not allowed waits for the
                          # user (the default elsewhere); "auto": yolo with the
                          # hazard guard (design §2.28); "allow": everything
                          # runs (yolo); "off": no shell tools
allow = ["cargo test"]    # command prefixes, word by word, added to the defaults
writable = ["~/devel/tmp"]  # more directories sandboxed shells write
user_shell = false        # true: the user's own bash or zsh, with their rc files

[shell.places."project:p-42ada59c2195414f"]
allow = ["cargo build"]   # only there; "always" answers add here (keys: `stitch attention`)
```

### The sandbox

With `approval = "sandbox"` a thread's shells run in an OS sandbox, and
commands don't ask. They write only in:
- the thread's place: its checkout, with the git directory and every worktree
  (`<checkout>-<name>` beside it too), or its folder;
- temporary directories;
- `~/.cargo`, `~/.rustup`, the cargo target directory (`CARGO_TARGET_DIR` or
  `build.target-dir`), `~/.cache`, `~/.npm`, `~/go` and, on macOS,
  `~/Library/Developer` and `~/Library/Caches`;
- the records in the node's data directory (the `stitch` record commands
  write them), not its configuration, keys or sign-ins;
- `[shell] writable`.

They read everywhere, reach the network, and can only signal processes they
started. `write_file` and `edit_file` write without asking wherever the
sandbox would. A write anywhere else fails with "Operation not permitted"
(macOS) or "Permission denied" (Linux).
The model is told to rerun such a command with `outside: true`, which runs
it in an unsandboxed session ("outside") under the rules below.

What reaches beyond the sandbox still asks:
- pushing and publishing: `git push`, `gh` (except its reading subcommands),
  `cargo publish`, `npm publish`;
- remote shells and uploads: `ssh`, `scp`, `rsync host:`, `curl -d`/`-X POST`;
- throwing away uncommitted work: `git reset --hard`, `git checkout -- …`,
  `git clean -f`, `git stash drop`, `git branch -D`, or `rm -r` of `.`, `*`,
  home, a `.git` or a checkout;
- programs that act outside the sandbox for the command: `sudo`, `open`,
  `osascript`, `tmux`, `docker run`, `launchctl`, `stitch yolo`/`answer`.

`always` grants those per place, as below (`git push`). On macOS,
`xcodebuild` and `swift` build inside the sandbox: the session turns off their
own sandboxes, which macOS cannot nest. On Linux the shell helper applies
Landlock. Kernels before 6.12 don't confine signals, so `kill` asks there.
Where shells can't be sandboxed, `sandbox` behaves as `ask`, and the node's
log says why.

### Unsandboxed sessions

A command line runs without asking when every command in it does. The line
is split at `;`, `&&`, `||`, `|` and newlines, minding quotes. `VAR=value`
in front of a command, `cd`, `pushd` and `popd`, and `git -C <dir>` are
fine. Redirection is fine only from files and to `/dev/null` or another
descriptor (`2>&1`). `for`/`while`/`if` blocks, `$(…)`, backticks,
`<(…)`, subshells and `{ …; }` groups run when every command in them does,
and so do `xargs`, `timeout`, `env` and `time` running an allowed command.
Writing redirections and here-documents always ask.

Allowed by default:
- `ls`, `pwd`, `cat`, `head`, `tail`, `wc`, `rg`, `grep`, `echo`, `printf`,
  `which`, `type`, `date`, `whoami`, `id`, `hostname`, `uname`, `uptime`,
  `file`, `stat`, `du`, `df`;
- `find`, `fd`, `tree`, `sort`, `uniq`, `cut`, `tr`, `nl`, `tac`, `rev`,
  `column`, `comm`, `diff`, `cmp`, `jq`;
- `xxd`, `od`, `hexdump`, `shasum`, `sha256sum`, `md5`, `md5sum`, `b3sum`;
- `realpath`, `readlink`, `dirname`, `basename`, `test`, `[`, `true`,
  `false`, `ps`, `printenv`, `[[`, `seq`, `expr`, `nproc`;
- `sed` and `awk`/`gawk` in their printing forms, `pgrep`, `free`, `lsblk`,
  `lscpu`, `journalctl`, `nvidia-smi`, `ip`, `ss`, `getent`, `strings`,
  `unzip`, `zipinfo`, `pdftotext`, `pdfinfo`, `systemctl`, `sw_vers`, and
  `--version` of `cargo`, `rustc`, `node`, `python3` and the like;
- `docker`/`podman` `ps`, `images`, `inspect`, `logs`, `version`, `info`;
- `gh pr view|list|diff|checks|status`, `gh issue view|list|status`,
  `gh auth status`, `gh run view|list`, `gh repo view`,
  `gh release view|list`, `gh workflow view|list`, `gh status`;
- `git status`, `log`, `diff`, `show`, `branch`, `rev-parse`, `remote`,
  `ls-files`, `grep`, `blame`, `describe`, `shortlog`, `stash list|show`,
  `worktree list`, `for-each-ref`, `ls-tree`, `cat-file`, `merge-base`,
  `check-ignore`, `rev-list`, `name-rev`, `ls-remote`, `show-ref`, `reflog`,
  `range-diff`, `cherry`, `count-objects`, `config --get|--list`;
- `stitch --version`, `threads`, `thread`, `jobs`, `attention`, `show`,
  `ls`, `edges`, `id`, `peer ls`, `project ls`, `secret ls`, `var ls`,
  `node status`, `memory status`, `send`, `cancel`, `sync`, `mcp`.

Their writing forms still ask: `find -exec`/`-delete`, `fd -x`,
`rg --pre`, `sort -o`, `tree -o`, `uniq in out`, `xxd in out`, `sed -i`,
sed's `w`/`e` (and `s///w`/`e`), `-f` scripts of sed and awk, awk's
`system`, `getline` and redirected or piped `print`, gawk's `-i`,
`journalctl --vacuum-*|--rotate|…`, `nvidia-smi`'s settings, `ip` but its
show forms, `ss -K`, `unzip` but `-l`/`-t`/`-p`/`-Z`, `pdftotext` but to `-`,
`systemctl` but `status|show|cat|list-*|is-*`, `gh … --web` and
`--show-token`, `git branch -d` (or naming a new branch), `git remote add`,
`git reflog expire|delete`, `--output=`, and `stitch thread tools <id> +x`.
The model is told this list in `shell_run`'s description. Restart the node
after changing `[shell] allow` or `user_shell`; `[shell.places]` grows at run
time.

```sh
stitch mode           # the approval mode, in a line
stitch mode ask       # the default: in the sandbox (or the allowed set) runs, the rest asks
stitch mode auto      # everything runs, unless the hazard guard judges it could do
                      # severe damage that cannot be undone: that asks you
stitch mode yolo      # everything runs, unjudged
stitch yolo on|off    # the same as `stitch mode yolo|ask`
```

Saved as `[shell] approval` in config.toml (`"auto"`, `"allow"` for yolo),
at once, no restart. The TUI's `/ask`, `/auto` and `/yolo [on|off]` do the
same, and its status line says `auto` or `yolo`. Yolo and auto also cover
MCP calls (§5).

**A place's own mode.** `approval = "ask" | "auto" | "yolo"` in a place's
`.stitch/settings.toml` (§7; `"allow"` and `"default"` work too) is followed
by that place's threads instead of the node's mode, stricter or looser: a
project in auto on a node in ask, or one kept in ask on a node in yolo.
Without it, the node's mode applies. It is read at each check, so a change
applies at once (as `stitch mode` does to the loops that start after it).
`stitch mode` run inside such a place says the mode there, that the place
sets it, and the node's; `stitch mode <mode>` still changes the node's. The
TUI's status line shows the mode of the directory it was started in.
Read-only threads stay read-only, a node whose `[shell] approval` is
`"off"` keeps its shell off, and Stitch's fixers keep their own mode.
Mind that the file is committed with the repository: a checkout cloned from
someone else can set itself to `yolo`. Look at `.stitch/settings.toml`
before working in a repository you do not trust (design §2.28).

**Auto mode and the hazard guard** (design §2.28). In auto, what would run
is first judged by a decision model, Gemini's `gemini-decision-1` with your
`gemini` key: one question, "could this do severe damage that cannot easily
be undone?", over the command, your goal and the files it runs inside the
thread's place (the scripts it names, the `just`/`make` recipe, the npm
scripts, cargo's `build.rs` and the Rust sources changed in the last week,
what the thread wrote this turn; never anything that may hold secrets, and
nothing is run to find them). At 0.7 or above, or on a hard stop (`rm -rf
~`, `dd` onto a disk, a forced push to `main`, …), it asks you, the
question starting with the verdict; below, it runs. Only you answer it: a
thread's own `stitch answer` is refused. Plain reading commands skip it; a
check takes about 250 ms and $0.0001. **Without the model** (no key, down,
slow), a command that needs judgement does not run: the agent tells you to
switch to `/ask` or `/yolo`, or set a fallback model
(`stitch decide set --fallback <model>`, verdicts then marked
uncalibrated). Hard stops still ask and reading commands still run. In
auto, Otter, browser and schedule calls are not judged (the browser keeps
its own rules). It catches mistakes; the sandbox, not the guard, is the
boundary.

The guard also has a mode for Stitch's fixers and their triage, and one for
landing their protected changes, both off by default and set with
self-improvement's settings (asked of you when an agent runs them):

```sh
stitch issue settings --mode auto      # what would ask a fixer runs when it stays in its
                                       # sandbox, only discards local work, and the model
                                       # clears "severe damage?" and "more power than its fix?";
                                       # without the model, it asks you (fixers run unattended)
stitch issue settings --auto-land on   # a protected fix lands unasked when none of six
                                       # questions on its diff is flagged; you get a notice
stitch issue revert <issue>            # undoes a landed fix on main
```

(`mode`, `auto_land` in `[self_improve]` as a node's fallback.)

**Decisions** (design §2.29). The guard is one user of a general service:
closed questions (yes/no, a choice, a score) answered by the decision model
with calibrated probabilities, in about 200 ms for about $0.0001. Each use
is a named profile with its questions and thresholds: `guard.user`,
`guard.fixer`, `land` (the guard's), `triage` (the harness-issues triage's
pre-pass: duplicates, not a bug, not actionable, which next, as advice in
its message), `loose-ends` (whether a conversation waits on you; the quick
model judges where it is unsure, its own path), `tool` (agents' own
questions: the `decide` tool, Otter's `Decide`). When the decision model is
unavailable a call fails, unless you set a fallback model (none by
default; never used for `guard.fixer` and `land`, which ask you, nor
`loose-ends`). The node's own decisions have a daily cap ($0.50).

```sh
stitch decide                                   # every profile, its thresholds, the node's day
stitch decide set guard.user --threshold 0.8    # auto mode's threshold
stitch decide set triage --threshold duplicate=0.7
stitch decide set loose-ends --threshold 0.7    # its one question
stitch decide set guard.fixer --threshold authority=0.4
stitch decide set --fallback anthropic/claude-haiku-4-5   # every profile that takes one
stitch decide set triage --fallback none
stitch decide set --model gemini/gemini-decision-1 --daily 1.00
stitch decide try loose-ends "$(cat excerpt.txt)"
stitch decide try tool "Buy cheap watches!!" --yes-no "Is this spam?"
stitch decide log --profile triage -n 10        # what was decided here, for calibration
```

Settings are yours, synced (`decide-settings` records); run from a thread's
shell, `stitch decide set` asks you first. Every call is recorded on the
node that made it, in `<data>/decide/<YYYY-MM>.jsonl` (the input's hash,
length and first words with secrets taken out, never the input whole), and
paid by the thread that asked, else by the node's own day. Loose ends'
decision model is on unless `[loose_ends] decide = false`.

**Always.** An approval whose commands a prefix can cover also offers
`always`:
- the TUI's `a`, inline or in Ctrl+T;
- `a` at `stitch chat`'s prompt;
- `stitch answer <question> always`.

It allows the command, and from then on what the question lists (the
program, or the program and its subcommand: `cargo test`, `rm`), in that
project or folder, on the node that asked. The grant is saved under
`[shell.places]` and takes effect at once: other approvals waiting on the
same thing go too. `stitch attention` shows what each question would grant.

**Read-only threads.** `spawn` with `read_only`, or
`stitch chat --detach --read-only`, makes a thread whose commands and MCP
calls that the rules do not allow are refused at once, never asked, so it
never waits on you. Yolo does not change that, and what it spawns is
read-only too. Use it for surveys, inventories and reviews.

Every approval is a `question` record on the attention list, answerable from
any node:

- `stitch attention` lists them, each with its thread, the node that asked and
  how long ago. Answer with `stitch answer <question> y` or
  `stitch answer <question> n [why]`.
- In the TUI: Ctrl+T. The status line shows `⚑ N` while N wait, and a new one
  is told once in the conversation, however deep in spawned threads it was
  asked.
- The chat that asked also prompts inline: `y` or `n` in the TUI,
  `run[ <session>]: <command>` in `stitch chat`. The first answer wins, from
  wherever it comes; answered elsewhere, the prompt goes away.

Nothing is declined for want of an answer. A question waits until it is
answered, or until yolo is turned on (it then runs). It is withdrawn when the
loop that asked stops: interrupted, cancelled, or a thread that reported
without it.
- A client that leaves (a closed terminal, the TUI opening another thread)
  answers nothing. The turn goes on at the node, and the question stays.
- A thread that `wait`s for another one blocked on an approval is told so,
  once per question, so it can tell you.
- A question a node left open that nothing on it asks any more is withdrawn
  by that node: within a minute, and at start once its threads have resumed.

## 5. MCP servers

Servers are defined per node in `config.toml`; a checkout or a synced record
never defines one. Nothing is offered to a thread until something enables it
(§6).

```toml
[mcp]
enable = ["fetch"]                      # every thread with tools on this node

[mcp.servers.fetch]                     # stdio: a command the node starts
command = "/opt/homebrew/bin/uvx"       # full path: the node's PATH may lack Homebrew
args = ["mcp-server-fetch"]
allow = ["fetch"]                       # tools that run without asking

[mcp.servers.github]                    # streamable HTTP
url = "https://api.githubcopilot.com/mcp/"
key = "github"                          # `stitch key set github`: sent as Authorization: Bearer
headers = { "X-Extra" = "value" }       # literal headers, sent as given
tools = ["get_*", "list_*", "search_*", "create_issue"]   # all it offers, anywhere

[mcp.servers.db]
command = "~/bin/db-mcp"
args = ["--readonly"]
env = { DB_URL = "postgres://localhost/app", PATH = "/opt/homebrew/bin:/usr/bin:/bin" }
cwd = "~/devel/app"                     # default: the home directory
key = "db"                              # a stdio server gets its key in the environment…
key_env = "DB_TOKEN"                    # …under this name (required with `key`)
timeout_secs = 300                      # a call's longest wait (default 120, at most 3600)
startup_timeout_secs = 60               # starting and listing tools (default 30)
approval = "allow"                      # every call runs without asking
share = true                            # offered to the user's other nodes, run here (§8)

[mcp.projects."~/devel/app"]            # threads whose shells start here or below
enable = ["github__get_*", "github__list_*", "db"]
```

`[mcp.servers.<name>]` keys:

| Key | For | Meaning |
|---|---|---|
| `command` | stdio | Program: a path (`~` expanded; relative to `cwd`) or a name looked up on `PATH`: the `PATH` in `env` when given, else the node's. |
| `args` | stdio | Arguments. |
| `env` | stdio | Added to the node's environment. A `PATH` here is also used to find `command`. |
| `cwd` | stdio | Where it runs (default `$HOME`). |
| `url` | HTTP | Streamable HTTP endpoint. The old HTTP+SSE transport is not supported. |
| `headers` | HTTP | Extra headers, literal values. |
| `key` | both | Name of a key stored with `stitch key set <key>` (or `STITCH_<KEY>_API_KEY`). HTTP: the bearer token. Stdio: the value of `key_env`. |
| `key_env` | stdio | The environment variable that gets `key`. |
| `tools` | both | Unprefixed tool names or `*` patterns: what the server offers at all (empty: everything). |
| `allow` | both | Unprefixed tool names or `*` patterns that run without asking. |
| `approval` | both | `"ask"` (default) or `"allow"` (every call runs). |
| `timeout_secs` | both | A call's longest wait, default 120, clamped to 1…3600. When it runs out, `notifications/cancelled` is sent. Progress notifications do not extend it. |
| `startup_timeout_secs` | both | Starting plus `tools/list`, default 30. |
| `share` | both | `true`: published to the user's other nodes; their calls run here (§8). |

Give a server either `command` or `url`, never both.

How servers run:

- Tool names are `<server>__<tool>`. Characters outside `[A-Za-z0-9_-]`
  become `_`, and a name is at most 64 characters; a longer one is left out,
  with a log line.
- A server starts the first time a turn of a thread that enables it needs its
  tools. Every thread on the node shares it. After a crash it starts again,
  backing off 2 s up to 60 s. It stops with the node, along with its whole
  process group.
- When a server's definition changes, the server is replaced at the next use.
- `[shell] approval = "allow"` (yolo) also lets every MCP call run without
  asking. Server annotations such as `readOnlyHint` allow nothing.
- An approval shows as `run [mcp:<server>]: <server>__<tool> <arguments>`.

**Full paths.** A stdio `command` is resolved against the node's `PATH`, not
the terminal's. Two cases leave `/opt/homebrew/bin`, `~/.cargo/bin` and
`~/.local/bin` out of it:

- the node was started by launchd or systemd;
- the node was started from a non-login shell.

So write `command` as a full path (`command -v uvx` tells it). Wrappers such
as `npx` and `uvx` start further programs: when those must be found too, put
a `PATH` in `env`.

**Verify:**

```sh
stitch mcp
```

For each server, `stitch mcp` starts it, lists its tools and stops it. A
server's line is `<name>: N tool(s)`, followed by `, shared with your other
nodes` when it is shared. Each tool line is `  <server>__<tool>`, marked
`[enabled here]` when threads in the current directory get it and
`[runs without asking]` when `allow` or `approval` covers it. A server that
failed prints `<name>: <error>`.

`stitch mcp` runs in the CLI process, with this terminal's environment, not
the node's. A server that passes here can still fail in the node (a `PATH`
difference), so also check the node's log after the first real use:

```sh
grep -E '^mcp' ~/.stitch/node.log | tail -n 30
```

**Logs.** A server's stderr goes to `<data>/node.log`, one line at a time.
Node log lines about MCP:

| Log line | Meaning |
|---|---|
| `mcp <name>: <line>` | The server's own stderr. |
| `mcp <name>: <why it did not start>` | A failed start. |
| `mcp <name>: the server stopped; starting it again` | Restart after the server exited. |
| `mcp: server name `<name>` left out: use a-z, 0-9 and -` | Invalid server name. |
| `mcp: enable: <entry error>` | An `enable` entry that does not parse. |
| `mcp <name>: tool `<tool>` left out: …` | Tool name too long, or taken. |
| `mcp: <config error>; using the configuration the node started with` | `config.toml` does not parse. |

Common failures (the error text is what threads and `stitch mcp` see):

| Message | Cause, fix |
|---|---|
| `` `uvx` is not on the node's PATH: give its full path, or PATH in `env` `` | Give the full path in `command`. |
| `<path>: No such file or directory (os error 2)` | The full path is wrong. |
| ``no key `github`: store it with `stitch key set github` `` | Store the key. |
| `` `key` for a stdio server needs `key_env`, the variable to put it in `` | Add `key_env`. |
| `give it either `command` (stdio) or `url` (streamable HTTP)` | Neither or both are set. |
| `MCP server <name> did not start: it did not start within 30s` | Slow first run (`uvx` and `npx` download packages). Raise `startup_timeout_secs`, or run the command once by hand. |
| `MCP server <name> is not running (<why>); next try in Ns` | Backing off after a failure. Fix the cause; it is tried again on the next use. |
| `MCP server <name> did not answer within Ns; the call was cancelled` | Raise `timeout_secs`. |
| `MCP server <name> stopped during the call (…); the call may or may not have taken effect` | The server crashed mid-call. The call is not retried. |
| `header <k>: …` | A header name or value is not valid HTTP. |
| `this call needs the user's approval, and nobody is here to give it` | No client can ask the user. Add the tool to `allow`. |
| `the user declined this call[: why]` | The user declined it. |

Not supported: OAuth (static keys and headers only), resources, prompts,
sampling, elicitation, the HTTP+SSE transport, and MCP on the phone.

## 5a. Tools written in Otter

A skill's scripts in its `tools/` folder (`*.otr`, the extension for new
ones; `*.otter` and `*.shs` work too) are tools (design §2.17): the home
workspace's skills (`<home>/skills/<name>/tools/`), a place's
(`<place>/.stitch/skills/<name>/tools/`) and Stitch's own (`google-flights`).
They are offered while their skill is on for the thread (`stitch thread
skills last -google-flights` turns the flights tools off), on every node:
macOS, Linux, the iPhone and visionOS. On the phone a call that asks shows
in the app's approvals (the bell), where "Always" allows that tool from
then on in the thread's place; its files are confined to the place's copy
on the phone, or to the app's own files for a global thread.

Try a script before a thread uses it, with Otter's command (`cargo build
--release -p otter-cli` in `../otter`):

```sh
otter tool ~/.stitch/home/skills/units/tools/convert.otr                 # its definition, as JSON
otter tool ~/.stitch/home/skills/units/tools/convert.otr '{"km": 42}'    # a call: output, then its log
otter check ~/.stitch/home/skills/units/tools/convert.otr                # diagnostics only
```

A thread can do the same on any node, the phone included, with Stitch's
`otter` skill (on by default; `stitch thread skills last -otter` turns it
off): `otter_search` and `otter_describe` look up shards, `otter_check`
checks code or a file in the thread's directory (`file:
".stitch/skills/units/tools/convert.otr"`) and says what effects running it
has, and `otter_run` runs code and prints what `otter run` would. A run
with no effects runs at once; one with effects (`Log` included: it is I/O)
asks as a tool does, shown as `run [otter]: otter_run <code>`, and follows
`[otter] allow` (`"otter_run"`), "always" answers, yolo, the time limit and
the thread's directory as tools do; a read-only thread is refused it.

The node compiles a script when a turn first offers it, and again when it, a
file in its `tools/` folder or a file it includes changes. The catalog is the
`otter` command's: core shards, `Http.*`, `FS.*`, and the text shards
(regular expressions, hashes). A file that defines no tool (a library others
`@include`) is skipped quietly.

```toml
[otter]
allow = ["google_flights_*"]   # patterns over tool names: run without asking
timeout_secs = 120             # a call's longest run (default 120, at most 3600)

[otter.places."project:p-42ada59c2195414f"]
allow = ["deploy_status"]      # added there by "always" answers (or by hand)
```

The language was called Shards until 2026-10-17: a `[shards]` table (and
`[shards.places]`) is renamed `[otter]` the first time the node reads
config.toml, the rest of the file kept, and `node.log` says so.

| What you see | Why |
|---|---|
| no tool offered, `otter: <path> is not offered:` and diagnostics in `node.log` | The script does not compile, or its definition is not a tool's. `otter check` shows the same. |
| `otter: skill <s>'s tool <name> is not offered: a name is …` | A name with `__`, outside `[A-Za-z0-9_-]`, over 64, or a built-in tool's. |
| `otter: skill <s>'s tool <name> is not offered: skill <t> has one of that name` | Two skills define the same tool; the first by skill name wins. |
| `run [otter:<skill>]: <tool> {…}` | It has effects (network, files, clock, waiting): approve it, or `always`. Pure tools never ask. |
| `this thread is read-only: <tool> is not among the Otter tools …` | Read-only threads run only what `[otter] allow` or a grant covers, files read-only. |
| `the tool call timed out after Ns` | Raise `timeout_secs`. What it logged follows. |
| `run [otter]: otter_run <code>` | A thread runs Otter code with effects (below): approve it, or `always`. |
| `Google Flights returned no usable data: …` | Google throttled the requests, or the network failed. Wait a minute or two. |

## 6. Tool and skill sets per thread

Every thread has a tool set and a skill set: an ordered list of entries. The
last entry that matches an item decides. Changes made on any node follow the
thread to every node and apply from its next turn.

Entry syntax:

- `+x` or `x` turns `x` on; `-x` turns it off. A name may use
  `[A-Za-z0-9_.*-]` and is at most 128 characters; `*` matches any run of
  characters.
- **Tool entries:**
  - A built-in group (`GROUPS` in `crates/stitch-node/src/toolset.rs`):
    `shell` (the `shell_*` tools, `watch_file`, `unwatch`, `jobs`,
    `job_stop`, `job_output`), `web` (`browse_web`, `web_eval`,
    `search_web`), `viewer` (`view_file`, `zoom_image`), `files`
    (`write_file`, `edit_file`, `commit_push`), `weave` (`spawn`, `send`,
    `wait`, `threads`, `cancel`, `models`, `schedule`, `wake_at`, `notify`,
    `report_issue`; `spawn` names it too), `memory` (`recall`), `images`
    (`generate_image`, with a Gemini key), `apps` (`show_app`), `browser`
    (the `browser_*` tools, while a Chrome profile is connected), and
    `self_improve` (off unless turned on by name; an agent cannot hand it
    on with `spawn`).
  - An MCP server (`github`: everything it offers).
  - A pattern over namespaced MCP tools (`github__get_*`).
  - A bare name is a group when it names one, else a server. `<name>__*`
    always means the server, for a server named like a group (`memory__*`).
- **Skill entries:** a skill's name, or a pattern (`-*` then `+mail`: only
  mail).

Layers, in order (later wins):

1. The built-in default: groups on except `self_improve`, MCP tools off,
   every listed skill on.
2. The node's `[mcp] enable` and `[mcp.projects."<dir>"] enable`. A bare name
   in these lists is always a server.
3. The place's `.stitch/settings.toml` (§7).
4. The thread's own changes, in the order written.

Commands:

```sh
stitch thread tools last                      # list: state, name, (on <node>), what it is
stitch thread tools last +github -web         # change, then list
stitch thread tools <id> +github__get_* -github__create_*
stitch thread skills last                     # list skills
stitch thread skills last -release +mail
stitch chat --tools -shell,+fetch "…"         # a new thread with changes (comma-separated; fetch: the MCP server of §5)
stitch chat --skills -release "…"
stitch chat --detach --tools +github "…"      # a detached one
```

`last` is the latest thread of the current directory's place, else the latest
global one. A listing prints `tools of thread <id>:`, then one line per item:

- `on ` or `off`;
- `on, unavailable here:` for an item that is on but cannot run on this node;
- then the name, `(on <node>)` for a server or skill that is another node's,
  and a note.

The note is one of: the group's tools, `MCP server`, `MCP server, shared`,
`MCP server, not shared by <node>`, `MCP tools matching`, or
`no MCP server of that name on this node`.

In the TUI, `/tools` and `/skills` take no arguments. They open an overlay
listing every item; Enter turns the selected one on or off, and Esc closes it.
For a new thread, the changes are sent with its first message. The overlay
toggles listed items only; write patterns with `stitch thread tools`.

`spawn` (the model's tool) takes `toolset` (an array of tool entries) and
`skills` (an array of skill entries). It writes them before the thread
starts, wherever it starts. A spawned thread starts from its own place's
defaults; it does not inherit its parent's changes.

A call to something that is off fails, and the result says how to turn it
on: "/tools in the TUI, or `stitch thread tools <thread> +x`". A call to a
server that another node configures without sharing says "… which does not
share it with the user's other nodes (`share = true` in its
[mcp.servers.<server>] there would)".

## 7. Places: settings and skills of a folder or checkout

A thread belongs to a place:

- a project, which is every git checkout, known by its first commit;
- a folder, where the same `~/…` path is the same folder on every node;
- or no place (global).

A place can say what its threads get, in `.stitch/` at its top: the
checkout's top, or the folder itself.

```toml
# <place>/.stitch/settings.toml  (unknown keys are an error)
enable = ["mail"]           # tool entries (§6): a server this node or another configures, a group, a pattern
disable = ["web"]           # tool entries turned off
approval = "auto"           # the approval mode here, over the node's (§4): ask, auto or yolo

[skills]
disable = ["release"]       # skills not wanted here
```

```
<place>/.stitch/skills/<name>/SKILL.md
```

`SKILL.md` starts with a `---` front matter holding `name` and `description`:

```markdown
---
name: mail
description: Handle the user's mail: triage, answer, archive, with the mail__* tools.
---
How to handle mail here: …
```

How the front matter is read:

- Values can be plain, `"quoted"`, or a folded block (`>` or `|` followed by
  indented lines).
- Without `name`, the directory's name is used.
- Threads see the list of names and descriptions, and read a skill when a
  task fits it.

What applies where:

- **Settings** only turn on what some node configures. A cloned checkout
  starts nothing new, and calls still go through approvals, though its
  `approval` may loosen them (to `auto` or `yolo`) for its own threads.
- **Place skills** are listed only for that place's threads. A place skill
  with a home-workspace skill's name replaces it there.
- **Home-workspace skills** are `<data>/home/skills/<name>/SKILL.md` (the home
  workspace's directory, wherever `stitch setup` put it), in the same format,
  listed for every thread of that node. With them are `AGENTS.md` and
  `agents/root.md` (the root's instructions). These are plain files of the
  node that runs the turn. Record sync does not carry them: copy them to each
  node, or keep the home workspace's directory in git. A skill another node
  must see (the phone) belongs in a place's `.stitch/skills/`.

**How places sync.** The node that has the place's directory reads its
`.stitch/` at each turn. When the files differ from what that node last
published, it writes a `place` record into the home workspace with:

- the place's key: `folder:~/Mail`, or `project:<repo>`;
- the node's name;
- the settings;
- each skill's name, description and text, at most 64 KiB each.

A node without the directory, or whose copy has no `.stitch/`, uses the
latest `place` record from any node. There, a place skill is listed with
"read it with `read_skill`", a tool that returns the skill's text (files next
to `SKILL.md` are not carried).

A `settings.toml` that does not parse is said in the node's log
(`place <key>: <error>`). The node then uses the latest record instead.

`[mcp.projects."<dir>"] enable` in `config.toml` still works. It is the
node's own per-directory default and does not sync.

Verify on the node that has the directory, then on another one after
`stitch sync`:

```sh
cd <place> && stitch thread tools last       # place settings applied
stitch ls --kind place                       # place records in the home workspace
```

### Folder-file sync: consent, limits and history

Folder identity links and place-settings sync are not the same as replicating
ordinary files. Outside git, Stitch can sync a folder's working files across
nodes. A new folder must be allowed first; the first thread can ask through
attention, without blocking the conversation. Yolo does not answer this grant.
Review the folder before allowing it: its contents will reach your trusted nodes.

```sh
cd ~/notes                         # an ordinary folder, not a git checkout
stitch folder sync --allow          # allow and sync now
stitch folder log                   # snapshot history, node and originating thread
stitch folder diff                  # latest change
stitch folder sync --stop           # stop further file sync
```

Stopping sync does **not** erase snapshots or copies already shared. Allowed
folders sync before and after turns and after peer sync. Git checkouts are not
replicated this way; each desktop's checkout remains its own.

Default limits are 2,000 carried files, 250 MB total, and 25 MB per file. A folder
above either aggregate limit syncs nothing, not a partial snapshot. Larger files,
symlinks and nested checkouts are not carried. Common build/cache directories are
ignored. Put additional patterns in `<folder>/.stitch/ignore`, one per line:
`*`/`?` match names, or relative paths when a pattern contains `/`.

```toml
# <data>/config.toml; restart after editing
[folders]
max_files = 2000
max_mb = 250
max_file_mb = 25
```

Text changes merge three ways when they do not overlap. Otherwise the later
snapshot wins and the other version is kept as a conflict copy; binary conflicts
also keep a copy. An edit against a deletion keeps the edit. Avoid storing
credentials in a synced folder: ignore rules and size limits are not a secret
scanner. See design §2.16 for snapshot and conflict semantics.

For other features, see [schedules and wake-ups](features.md#schedules-and-wake-ups),
[SSH](features.md#secrets-variables-and-ssh), [browser setup](features.md#browser-control-your-own-chrome),
[apps and grants](features.md#apps), and the [complete command overview](commands.md).

## 8. Worked example: `~/Mail` with Mail.app, on the Mac and from other nodes

Goal: conversations in `~/Mail` get a Mail.app MCP server and a mail skill.
The server runs on the Mac (`laptop`). Threads started on another node use it
through the Mac. That node is a Linux box (`nas`) here; the iPhone's node
uses it the same way.

Stitch does not ship a Mail.app server. Use any stdio MCP server that drives
Mail.app; below, `<mail-server>` stands for its executable and `<args>` for
its arguments.

**1. On the Mac, define and share the server.** Add to `~/.stitch/config.toml`:

```toml
[mcp.servers.mail]
command = "/opt/homebrew/bin/<mail-server>"   # full path (`command -v <mail-server>`)
args = [<args>]
allow = ["search*", "list*", "get*", "read*"] # read-only tools run without asking; match the server's names
timeout_secs = 180
share = true
```

Do not add `mail` to `[mcp] enable`: the folder enables it (step 3).

**2. Check it starts.**

```sh
stitch mcp
```

Expect `mail: N tool(s), shared with your other nodes` and the
`mail__…` tools. The first time it drives Mail.app, macOS asks for Automation
permission (System Settings → Privacy & Security → Automation). Allow it for
the program that runs the server. If the node was started from a terminal,
the request may name that terminal.

**3. The folder's settings and skill.**

```sh
mkdir -p ~/Mail/.stitch/skills/mail
cat > ~/Mail/.stitch/settings.toml <<'EOF'
enable = ["mail"]
EOF
cat > ~/Mail/.stitch/skills/mail/SKILL.md <<'EOF'
---
name: mail
description: Handle the user's mail with the mail__* tools: triage, draft replies, archive.
---
Search before reading whole threads. Never send without the user's approval:
draft, show the draft, and send only when asked.
EOF
```

**4. Publish the server to other nodes.** Restart the node, or start one
turn, so it starts its shared servers and writes its `capabilities` record.
Then sync:

```sh
stitch node stop && stitch jobs
cd ~/Mail && stitch chat "List the mail tools you have, without calling them."
stitch sync
```

**5. Verify on the Mac.**

```sh
cd ~/Mail
stitch thread tools last         # "on  mail … MCP server"
stitch thread skills last        # "on  mail … Handle the user's mail…"
stitch chat -t last "How many unread messages are in my inbox?"
```

The reply should come from a `mail__…` call. A tool outside `allow` asks
first: `run [mcp:mail]: mail__<tool> {…}`.

**6. Verify from another node** (`nas`). The same `~/Mail` path is the same
folder there. The directory need not hold anything, but the shell must start
somewhere:

```sh
stitch sync
mkdir -p ~/Mail && cd ~/Mail
stitch chat "How many unread messages are in my inbox?"
stitch thread tools last         # "on  mail (on laptop)  MCP server, shared"
stitch thread skills last        # "on  mail (on laptop)  Handle the user's mail…"
```

On `nas`, the thread sees the tools as `mail__…`, described as
`MCP server mail on laptop: …`. Its calls run on the Mac under the Mac's
`allow`, `approval` and yolo. A call that needs approval becomes a question,
answerable from either machine (`stitch attention`, `stitch answer`). The
skill is read with `read_skill`.

When the Mac cannot be reached within 5 seconds, a call fails at once:
`mail__… runs on laptop, which cannot be reached now (…); nothing was run. To run it when laptop is back, call queue_call …`.

The thread can then queue the call (`queue_call`). It runs after the Mac's
next sync, and its result comes back as a report:
`[Report <id> from laptop: queued call mail__<tool> (<call>)]`.
A queued call's report carries text only.

If `nas` lists `mail` as `on, unavailable here: … not shared by laptop`, then
`share = true` is missing on the Mac, or the Mac has not written its
`capabilities` record since it was added (step 4). Then:

```sh
stitch ls --kind capabilities    # on nas: the Mac's latest capabilities record
```

## 9. Detached threads and budgets

```toml
[detached]
max_steps = 200       # model calls a turn takes at most (default 50); a spawn can set its own
keep_shells = false   # true: a thread's shell sessions outlive its turn
```

Restart the node after changing these.

```sh
stitch chat --detach "update the dependencies and run the tests" \
  --budget-usd 2 --budget-tokens 2000000 --minutes 30 --max-steps 120
stitch chat --detach --on nas --context --parent <thread> "implement what we discussed"
stitch chat --detach --no-tools "a second opinion on …"
```

`stitch chat --detach` options:

- `--on <node>`: run it on another node.
- `--context [this|none|<thread>]`: the conversation it starts from. Bare
  `--context` means `this`: `--parent`'s conversation so far.
- `--parent <thread>`: where its report goes. It defaults to `STITCH_THREAD`,
  which is set in a thread's shells.
- `--no-tools`: no tools; the thread only answers.
- `--tools`, `--skills`: changes to its sets (§6).
- `-m <provider/model>`: its model.
- `--global`: a thread that belongs to no place.
- `--read-only`: it only looks; what the rules do not allow is refused,
  never asked, so it never waits on you (§4).
- `--memory`: memory remembers it. Without it a detached thread is out of
  memory from its start, though it may still `recall` what other
  conversations left.
- `--no-wake`: its report is kept for you on `--parent`'s thread and does
  not wake it.
- `--budget-usd`, `--budget-tokens`, `--minutes`, `--max-steps`: below.

`stitch send <thread> <text>` continues any thread: `--from` names the
thread the reply is reported to (default `STITCH_THREAD`), `--no-wake` keeps
that report for you, and `--interject` weaves the message into the turn
running now, between its steps, instead of waiting for it to end.
`stitch thread <id> --read-only` / `--writable` switches an existing
thread (only you can; agents cannot).

A detached thread takes no `--thread` or `--attach`.

The `spawn` tool takes `goal` (required), `node`, `model`, `tools` (a boolean),
`context` (`none`, `this` or a thread ID), `budget_usd`, `budget_minutes`,
`max_steps`, `toolset` and `skills`.

How budgets work:

- Budgets are per thread, spent over all its turns. A turn woken later gets
  what is left; with nothing left, it does not run.
- A spawned thread gets at most what its parent has left.
- `max_steps` is per turn. The turn's own value comes first, else
  `[detached] max_steps`, else 50.
- When a detached thread's turn ends, its jobs end, and its shell sessions
  close unless `keep_shells = true`.

Watch and steer:

```sh
stitch threads           # [running], [waiting], [done], [over-budget], …
stitch jobs              # shell jobs here and detached threads on every node; `stitch jobs stop <id>`
stitch attention         # its approval questions
stitch answer <question> y   # or: n [why]
stitch send <thread> "and add a test" --from <thread>
stitch cancel <thread>
stitch thread <thread>   # its transcript
```

Notifications on your phones: agents send them with the `notify` tool when
they have news you would want now (say so in a schedule's goal: "notify me
when the build breaks"); at most 10 per thread an hour. From a script or a
shell:

```sh
stitch notify "backup finished: 312 GB" --title "nas"
long-build && stitch notify "build done" || stitch notify "build failed"
```

In a thread's shell it names that thread (`STITCH_THREAD`), so tapping the
notification opens its chat. They go the way approvals do (`stitch apns
status`); the phone's own threads show theirs as local notifications.

## 10. Memory

```sh
stitch memory install            # EmbeddingGemma 2 into <data> (~310 MB, +555 MB for images); --text-only
stitch memory status             # "memory on: N facts folded, M episodes", or "memory off: run `stitch memory install`"
```

Install once per machine. If the node is running, `install` stops it; memory
starts with the next command.

```toml
[memory]
enabled = true          # false: no memory on this machine
images = true           # false: text only
gpu = true              # false: encode on the CPU
gist_model = "anthropic/claude-haiku-5-5"   # writes gists; quick when unset
```

Per thread: `stitch memory off [thread]`, `stitch memory on [thread]` (the
latest of this place by default), or `/memory on|off` in the TUI.

**Recalibrating the 0.49 gate.** An unasked memory reaches a message only
within 0.49 cosine distance of the cue, with both embeddings centered. The
constant is `stitch_memory::AMBIENT_CENTERED_DISTANCE`. It is compiled in,
with no config or environment override yet, and it is provisional (design
§2.9).

To collect the data:

```sh
stitch memory notes > notes.tsv
```

`stitch memory notes` prints one row per memory shown in an unasked note,
tab-separated:

```
note  handle  raw  centered  cue_distinctive  cue_chars  novelty  ceiling  wandering  fresh  outcome
```

`outcome` is `helped`, `dismissed`, `ignored` or `unknown`.

To recalibrate:

1. Wait until a few hundred rows are `helped` or `dismissed`.
2. Compare their `centered` distances (and `raw`, by `cue_chars`).
3. Put the cutoff where dismissals take over.
4. Replay the field threads offline to see what the new cutoff shows and
   drops: each substantive user message as a cue, against the earlier
   memories of other threads.
5. Change the constant, rebuild, and run `stitch memory audit`, the drift
   check.

The rows only cover memories that passed the gate. Near-misses are not
logged, so the data says where to tighten, not where to loosen (design §4).

## 10a. Your instructions

```sh
stitch instructions                      # yours for this project or folder
stitch instructions --edit               # edit them in $EDITOR
stitch instructions --global --edit      # yours for every thread
stitch instructions --set rules.md       # from a file (`-`: standard input; empty: none)
```

What you want of agents beyond the repository's own `AGENTS.md`, without
committing it: synced to every node (the phone edits them in Settings and a
chat's menu), told to threads after `~/.stitch/home/AGENTS.md` and before the
checkout's files.

## 10b. Threads: search, titles, pins, forks, summaries and compaction

```sh
stitch search terrarium misting          # every thread's messages, best first
stitch search --here -n 50 the sync fix  # only this project's or folder's threads
stitch rename <thread> Records format    # a title, on every node
stitch pin <thread>                      # first in lists (--off unpins)
```

Search ranks messages by their words (BM25, every thread) and, where memory
runs, by their meaning (EmbeddingGemma 2, the threads memory holds), fused by
reciprocal rank. Each hit names its thread, its message (`m12`, for
`stitch fork --upto`) and what found it. In the TUI, `/search [--here] <words>`
lists the hits, and Enter opens the thread.

A new chat is titled from its first exchange by `[threads] title_model`, else
the quick model, in the background; `titles = false` turns that off. A title
you give (`/rename`, `stitch rename`) stands. `/pin` in the TUI pins or unpins
the open thread, and pinned threads show 📌. `/quote` starts a new thread with
the open one attached as a Markdown file, framed as quoted material to ask
about, not to continue. After each turn, the status line shows its cost and
the thread's total (API keys only).

```sh
stitch fork last                         # a new thread continuing the latest here
stitch fork <thread> --upto m12          # up to a message (`stitch thread` shows them)
stitch fork <thread> --last 6 --no-tools --no-attachments
stitch fork <thread> --summarize         # from a summary of it
stitch fork <thread> --summarize --last 10   # a summary, then its last 10 messages
stitch chat -t <new> "go on"             # its first message
```

In the TUI:

| Command | Does |
|---|---|
| `/fork [N] [-a] [-t]` | Fork all of the thread or its last N messages; `-a` leaves attachments out, `-t` tool calls |
| `/summarize` | Summarize & continue |
| `/compact [N]` | Summarize & truncate: a summary, then the last N messages (10) as they were |

Each opens the new thread. Its transcript starts with what it continues and
the summary.

A fork reads the conversation from the thread it continues; nothing is
copied. It takes the thread's system prompt, place, and tool and skill
changes. Titles start with ⑂ (fork), ↻ (summary) or ✂ (summary and the last
messages).

**Compaction.** A thread whose last step sent its model more than
`compact_at` percent of the model's context window continues in a new thread
before its next turn: a summary, then its last `compact_keep` messages.
- The client is told.
- A detached thread's new one keeps its goal, budget, model, policy and who
  it reports to.
- What is sent to the old thread, its reports, `wait` and `stitch cancel` all
  go to the new one.
- A prompt the provider refuses as too long is compacted the same way and
  tried again once.

```toml
[threads]
summary_model = "anthropic/claude-sonnet-5-5"  # summaries (default: quick, with continuity safeguards)
title_model = "anthropic/claude-haiku-5-5"     # titles (default: quick)
# These overrides and memory.gist_model also accept "default" or "quick".
titles = true                                  # false: no generated titles
compact_at = 80                                # percent of the window; 0: never
compact_keep = 10
```

Context windows are the provider's `context_window` when set. Otherwise the
node asks the provider once per model: Anthropic, Codex, xAI, OpenRouter,
llama.cpp (what it serves), Ollama (the loaded model's window, else its
`num_ctx`) and Gemini say. OpenAI's API does not, and a failed lookup falls
back too, to a guess by model name: Claude 1M (200k before 4.6 and Haiku
4.5), GPT-5.4 to 6 1.05M, other GPT-5 400k, o-series 200k, Grok 256k to 1M,
Gemini 1M, others 128k. Past 272k tokens OpenAI bills a whole request at
twice the input price (xAI past 200k); set `compact_at` lower, or
`context_window`, to stay under it.

## 10c. Runtime limits, scheduling and notices

These optional node-local settings are read at startup; restart after editing.
They are independent of a detached task's own remaining budget.

```toml
[chat]
max_steps = 200       # attached chat asks before taking this many more model calls

[detached]
max_steps = 50        # default calls per turn; a spawn can override it
keep_shells = false  # close shells when a detached turn ends
gather_secs = 5       # gather idle-thread reports before waking once (at most 60 s total)

[schedules]
default_node = "desktop"  # optional: new schedules without --on fire there
check_secs = 120          # longest no-model check

[loose_ends]
every_minutes = 60   # 0 disables the periodic pass
push = true          # notify when a conversation newly waits on you
model = "quick"      # model used to judge changed conversations

[notifications]
desktop = false      # optional desktop notifications (D-Bus / Notification Center)
```

Use the name of an actual paired always-on node for `schedules.default_node`,
or omit it to fire schedules here. A phone cannot be relied on to wake itself
for unattended scheduled work. Full scheduling workflow and budgets:
[feature guide](features.md#schedules-and-wake-ups).

`stitch loose-ends --list` reads saved judgments without invoking a model;
`stitch loose-ends` can spend on judging changed conversations. Disabling its
periodic pass does not disable explicit judging. Desktop notifications are
separate from [phone pushes](features.md#loose-ends-deletion-and-notifications).

## 11. Troubleshooting

**The node's log** is `<data>/node.log` (`~/.stitch/node.log`), appended to
by every background start. Under `stitch node`, the log goes to that
terminal. If the node exits right after starting, the command that started it
prints the log's last lines: `the node exited (…):`. Grep the log by prefix:

| Prefix | What |
|---|---|
| `mcp` | MCP servers, §5 |
| `shells:` | the shell helper |
| `session <thread>:<name> did not start:` | a shell that did not start |
| `place <key>:` | place settings |

**"the shell did not start: …"**: what the shell tools return, also logged as
`session … did not start: …`:

| Message | Meaning |
|---|---|
| `<shell> (pid N) was executed, then its output ended; the shell exited with status S; it printed "…"` | The shell died before its first prompt. With `user_shell = true`, the user's rc files are the usual cause (an `exit`, a failing command under `set -e`). What it printed says more. |
| `… then its terminal closed (EIO); the shell was killed by signal N (SIGx); …` | It was killed or crashed during startup. |
| `… ; the shell (pid N) had not exited; …` | Its output ended, but the process remains. |
| `no prompt within 10s; saw: …` | It started but never showed a prompt (a slow or interactive rc file). |
| `<shell> was not started within 10s (the shell helper did not answer)` | The helper hung. Check `ps` for `stitch-shell-helper`, and reinstall. |

Other errors from starting a shell:

| Message | Meaning |
|---|---|
| `the directory <dir> is not there` | The thread's place has no directory on this node. Create it, or chat from elsewhere. |
| `<program> is not in PATH (<path>)` | The shell program itself was not found. |
| `starting <program> (by the shell helper): <error>` | The helper could not exec the shell. |
| `the shell helper (<path>) did not start the shell` | The helper ended without saying why. |
| `allocating a terminal failed at <stage>: <error> (errno N)[, after N tries]` | No PTY. Too many open terminals, or a macOS PTY race past its retries. |

**Helper version mismatch.**

`the shell helper <path> is of another version (it speaks protocol N | no protocol: it is older, this node M): install stitch and its helper together (e.g. `cargo install --path crates/stitch-cli`), or point STITCH_SHELL_HELPER at a matching one`

To fix it:

1. Reinstall both binaries.
2. Check that `STITCH_SHELL_HELPER`, if set in the node's environment, names
   a current build.
3. Restart the node.

**Helper missing (macOS).** The node logs this once:
`shells: <path> is missing (…); shells start through /bin/sh, …`.
Shells still work. To fix it, install with `cargo install --path
crates/stitch-cli`, or set `STITCH_SHELL_HELPER`, then restart the node. Only
the log says so for now (design §4).

**`bash: child setpgid (A to B): Operation not permitted`** in a command's
output, now and then, on macOS. This is a race between bash and the job it
starts, both calling `setpgid`; it also happens in plain C. The job still runs
in its process group. It is harmless, and Stitch does not hide it (design
§2.3).

**An MCP server is not on the node's PATH.** The message is
`` `<cmd>` is not on the node's PATH: give its full path, or PATH in `env` ``.
`stitch mcp` may still pass, because it uses the terminal's `PATH`. Give the
full path in `command` (§5). The change applies from the next turn; no
restart is needed.

**A remote node is offline.** These messages mean it:

- `… runs on <node>, which cannot be reached now (…); nothing was run …` from
  a shared tool;
- `stitch sync` printing `<node> <repo>: timed out` or an error for that node;
- `stitch node status` still listing it under peers.

Check the other machine: is its node running (`stitch node status` there),
and does it have network? Work for it can wait meanwhile:

- `queue_call` for a shared tool;
- `stitch send` and `--on <node>` write records that the node takes when it
  syncs again.

When it is back, run `stitch sync <node>`.

**A thread's tools are not what you expect.** Run
`stitch thread tools <thread>` on the node that runs it. It shows every layer's
outcome; `on, unavailable here:` says what cannot run there.
