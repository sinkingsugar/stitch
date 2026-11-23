# CLI and terminal UI reference

This is the navigation reference for the public command surface. For exact
arguments, defaults and validation in your installed build, use:

```sh
stitch --help
stitch chat --help
stitch schedule add --help
stitch help ssh host add
```

The [full generated help reference](cli.md) includes every argument and default;
the [dedicated TUI guide](tui.md) expands on terminal interaction.

`--help` does not perform the operation. Hidden implementation commands (such as
`browser-host`) are not user interfaces. Sources of truth are the Clap definitions
in [`main.rs`](../crates/stitch-cli/src/main.rs) and
[`vault.rs`](../crates/stitch-cli/src/vault.rs), and the TUI palette in
[`tui/app.rs`](../crates/stitch-cli/src/tui/app.rs).

## Global options

| Option | Meaning |
|---|---|
| `--data <dir>` / `STITCH_DATA` | Node data directory; default `~/.stitch` |
| `--root <dir>` / `STITCH_ROOT` | Record root for record commands; default home workspace |
| `--node <name>` / `STITCH_NODE` | Override this node's name; **not** a remote-node selector |
| `-h`, `--help` | Help for the selected command |
| `-V`, `--version` | Version and build provenance |

`stitch` with no subcommand opens the TUI. Commands needing a node start it in
the background if necessary; status/help and direct-store operations need not.
Use command-specific `--on` for delegation or schedules, and `stitch model ls
--node <name>` for another node's models.

## Conversations and orchestration

| Command | Purpose / important options |
|---|---|
| `chat [text]` | New message; omitted text or `-` reads stdin. `-t <id\|last>` continues a thread, `-a <file>` attaches (repeatable), `-m <provider/model>` selects a model, `--global` selects no place, `--quiet` hides reasoning summaries |
| `chat --detach <goal>` | Autonomous task; `--on <node>`, `--parent <thread>`, `--context [this\|none\|id]`, `--no-tools`, `--no-wake`, `--read-only`, `--memory`; budgets `--budget-usd`, `--budget-tokens`, `--minutes`, `--max-steps` |
| `threads [--all]` | This place's and global threads, or all places |
| `thread <id\|last>` | Read a transcript; `--read-only` or `--writable` changes mode |
| `thread tools <id> [changes…]` | List/change tools, e.g. `+github -web` |
| `thread skills <id> [changes…]` | List/change skills, e.g. `-release +mail` |
| `thread rm [ids…]` | Delete across nodes after confirmation, with compaction chains. `--spawned-by <id>` also selects immediate children; `--yes` skips confirmation |
| `thread share <id>` | Share a conversation as a `.stitch` archive (a zip: transcript, attachments and app versions as blobs, a read-only `index.html`; design §2.31): secrets redacted, a leak scan listed and asked about first. `--effort` the whole spawn tree; `--level messages\|calls\|full`; `--out <file>`; `--dry-run`; `--yes` |
| `thread import <file>` | Import a shared `.stitch` file: verified (every entry's hash, the signature), previewed and asked about (`--yes` not), then written as foreign history with no tools until `stitch event untaint <id>` |
| `send <thread> [text]` | Continue on its owning node; `--from <thread>` receives the report, `--interject` weaves into a running turn, `--no-wake` keeps the report without waking the recipient |
| `cancel <thread>` | Stop a detached thread |
| `rename <thread> <title…>` | Set a synced title |
| `pin <thread> [--off]` | Pin/unpin in lists |
| `search <words…>` | Search messages; `--here` scopes to the place, `-n/--limit` limits hits |
| `fork <thread\|last>` | New continuation; `--upto m12`, `--last N`, `--no-tools`, `--no-attachments`, `--summarize` |
| `instructions` | Read your instructions; `--edit`, `--set <file\|->`, `--global` |
| `jobs [stop <id>]` | Background shell jobs here and detached threads across nodes; stop interrupts a command or cancels a thread |
| `shell attach <thread>/<session>` | Watch and type into an agent's terminal; Ctrl+] detaches |
| `attention` | Work and approval questions needing you |
| `answer <question> <y\|n\|always> [reason…]` | Answer an approval; inspect the grant before choosing `always` |
| `yolo [on\|off]` | Show/change ordinary shell approval bypass; saved per node |
| `notify [body]` | Phone notice; stdin when omitted/`-`, `--title`, `--thread` |
| `loose-ends` | Judge (the decision model, the quick model where it is unsure), then list; `--list` avoids judging, `--dismiss <thread>` hides until it changes |
| `mode [ask\|auto\|yolo]` | The approval mode: auto is yolo with the hazard guard (design §2.28); `yolo on\|off` is `mode yolo\|ask` |
| `decide [profiles\|set\|try\|log]` | Decisions (design §2.29): the profiles and their thresholds; `set [<profile>] --threshold [key=]p --fallback <model\|none> --model m --daily usd`; `try <profile> <text> [--yes-no "…?"]`; the node's log |

`chat --tools <comma-separated changes>` and `--skills <changes>` set thread
policy. `--memory` opts a detached task into being remembered; it can recall by
default without being remembered. Detached tasks cannot take `--thread` or
attachments. See [delegation](features.md#threads-that-start-threads),
[thread management](setup.md#10b-threads-search-titles-pins-forks-summaries-and-compaction)
and [approval rules](setup.md#4-shell-approvals).

## Schedules

| Command | Purpose |
|---|---|
| `schedule add <goal…>` | Create a schedule with `--at` or `--every`; see flags below |
| `schedule list [--all]` | Active schedules, or ended ones too |
| `schedule show <id\|name>` | Details, firings and their costs |
| `schedule next` | Next firings |
| `schedule pause <id\|name>` | Pause |
| `schedule resume <id\|name>` | Resume |
| `schedule run <id\|name>` | Fire now on the pinned node |
| `schedule rm <id\|name>` | End |

Creation supports `--name`, `--at`, `--every`, `--tz`, `--place`, `--on`,
`--model`, `--max-steps`, `--read-only`, `--budget-usd`, `--budget-tokens`,
`--budget-minutes`, `--daily-cap-usd`, `--check` or `--check-file`, `--fresh`,
`--no-catch-up`, `--reports-to`, and `--wake`. A schedule spends without an open
client: configure budgets deliberately. [Examples and check behavior](features.md#schedules-and-wake-ups).

## Node lifecycle and sync

| Command | Purpose |
|---|---|
| `setup --name <name>` | Name a node and create its home workspace; `--home`, `--no-home`, `--local` |
| `node` | Run in foreground |
| `node status` | Inspect the running node without starting one |
| `node stop [--when-idle]` | Stop now, or once idle |
| `node restart [--now]` | Restart once idle, or cut current work now |
| `node builds [--healthy-for <secs>]` | Installed/running builds, trial, health and failures |
| `node service install` | Install systemd user service / macOS LaunchAgent; `--print` previews; `--exe` selects binary |
| `node service install --update [checkout]` | Also install update timer; optional `--follow`, `--promote`, `--testflight [app checkout]` for rollout |
| `node service uninstall` | Remove service and update timer |
| `id` | Endpoint ID |
| `invite [--minutes N]` | Time-limited pairing ticket and QR code |
| `join <ticket> [--home <dir>]` | Pair and join home workspace |
| `sync [peer]` | Sync all trusted nodes, or one |
| `peer add <name> <id> [--addr <ip:port>]` | Trust a node manually; direct address repeatable |
| `peer ls` | Trusted nodes |
| `peer remove <name>` | Revoke trust across nodes |
| `project ls` | Known projects and local checkout paths |
| `project split` | Treat this fork as a separate project |
| `project rebind [--project <name\|id>]` | Have the project of this origin follow its repository, re-created from a new first commit |
| `folder link <node:path\|path> [--force]` | Link folder identity; `--force` permits an unknown location |
| `folder unlink` | Make this folder node-local in identity |
| `folder sync [--allow\|--stop]` | Sync files now; grant/revoke folder-file sync |
| `folder log [path] [-n N]` | File-change history |
| `folder diff [path] [--from <id>] [--to <id>]` | Diff changes |

See [installation and updates](setup.md#1-install-update-restart),
[pairing](setup.md#2-pairing-peers-sync), and [places](features.md#where-threads-belong).
Stopping a node ends its shell sessions and jobs. Detached threads resume on
restart; ordinary client chats do not have the same restart behavior.

## Providers, models and credentials

| Command | Purpose |
|---|---|
| `auth login <provider> [--file]` | Subscription sign-in (`anthropic`, `openai`, `grok`) or GitHub device sign-in (`github`) |
| `auth logout <provider>` | Remove local sign-in; API keys are separate |
| `auth status [provider]` | Inspect sign-in without printing tokens |
| `key set <provider> [--file]` | Read an API key without echo, or from stdin; alias for a provider-named secret |
| `key rm <provider>` | Remove provider secret |
| `secret set <name>` | Prompt/stdin; `--here` or `--scope`, `--device-only` or `--sync`, `--keep-value`, `--file` |
| `secret get <name> [--reveal]` | Presence by default; **plaintext** with `--reveal`; accepts scope |
| `secret ls` | Names, scope and placement, not values |
| `secret rm <name>` | Remove at scope; synced unless device-only |
| `var set <name> <value>` | Set nonsecret variable; accepts scope |
| `var ls` | List variables |
| `var rm <name>` | Remove at scope |
| `model ls [--node <name>]` | Models, prices and settings from this/another node |
| `model default [provider/model] [--unset]` | Read/set/clear synced default model |
| `model quick [provider/model] [--unset]` | Read/set/clear synced quick model |
| `model set <pattern>` | Set `--reserved [true\|false]`, `--price <input/output>`, `--note`; quote wildcard patterns |
| `model rm <pattern>` | Remove pattern settings |

Scope is global by default, `--here` for the current place, or `--scope
global|project:<repo>|folder:<key>`. Do not use a secret as a command-line
argument: prompted input avoids shell-history leakage. A subscription takes
precedence over an API key for its built-in provider at the official endpoint.
See [setup §3](setup.md#3-providers).

## SSH

| Command | Purpose |
|---|---|
| `ssh host add <name>` | Add/replace synced host; `--address`, `--port`, `--user`, `--jump`, `--auth`, `--approval`, `--no-tmux` |
| `ssh host ls` / `ssh host rm <name>` | List/remove hosts |
| `ssh key` | This device's public key (created on first use) |
| `ssh authorize <host> [--node <name>\|--key <public-key>]` | Add a device's key through a node that can connect |
| `ssh authorizations` | Requests and outcomes |
| `ssh hostkey ls` / `ssh hostkey rm <host>` | List/forget pinned host keys |

For authentication and trust choices, see [the SSH guide](features.md#secrets-variables-and-ssh).

## Tools, memory, browser and apps

| Command | Purpose |
|---|---|
| `mcp` | Start configured MCP servers, inspect offered/enabled tools, then stop them |
| `memory install [--text-only]` | Download memory encoder/projector on this node |
| `memory status` | State and counts |
| `memory audit` | Drift diagnostics |
| `memory notes` | TSV of ambient memory notes and outcomes |
| `memory on [thread]` / `memory off [thread]` | Include/exclude a thread from memory; latest here when omitted |
| `browser install` | Extension files and native messaging host; `--profile-dir` targets test profiles |
| `browser install headless` | Download Chromium for isolated web tools instead |
| `browser profiles [disable <name>]` | Inspect profiles, or disable one; enable from that profile's extension popup |
| `browser call <tool> [json]` | Run browser tool manually; `--thread`, `--images` |
| `app list` | Pinned apps and addresses |
| `app open [name\|id\|thread\|version]` | Open app, or app list |
| `app pin <thread\|version>` | Pin latest/fixed app; `--name`, `--icon` |
| `app unpin <app>` | Remove pin, not app versions |
| `app allow-live <app> [--thread <id>] [--deny]` | Grant/revoke live thread/job/approval reads |
| `app allow-threads <app> [--thread <id>]` | Preview needs; `--needs <hash>` grants; `--deny` revokes and cancels work |
| `app allow-container <app> [--thread <id>]` | Preview; `--build`, `--run`, `--shell` grant exact hashes; `--deny` revokes |
| `app storage [--rm <name>]` | Inspect/remove container images and caches, not app data |
| `dashboard` | Pin and open built-in live dashboard |
| `apns set <p8> --key-id <id> --team-id <team>` | Own-team APNs key; `--file` |
| `apns relay <url>` | Custom push relay |
| `apns test` / `apns status` | Test/inspect phone notifications |

See [features](features.md), [MCP setup](setup.md#5-mcp-servers),
[Otter tools](setup.md#5a-tools-written-in-otter), and [browser control](browser-tool.md).
App containers require Linux and rootless Podman. Browser control needs a
connected enabled desktop Chrome profile; it is separate from headless web reads.

## Integrations, issues and experimental vault

| Command | Purpose |
|---|---|
| `discord` | Bot front-end; guild allowlist, per-user turn limits, model, server instructions, approvers and announcements; token from environment/file |
| `proxy [serve] [--listen <address>]` | Serve budgeted API proxy; `--dir` / `STITCH_PROXY_DIR` selects config directory |
| `proxy token add <name>` | Print token once; `--daily-tokens`, `--daily-tokens-per-user`, `--rpm`, repeatable `--upstream` |
| `proxy token ls` / `proxy token rm <name>` | List/revoke client tokens |
| `proxy usage [--days N]` | Daily usage by token and end user |
| `issue list [--all]` | Open/fixing issues, optionally fixed/dropped |
| `issue show <id>` | Issue details |
| `issue report <title> --what <text>` | File; `--expected`, `--evidence`, `--area` |
| `issue set <id> [state]` | State/note/fix; `--note`, `--fix` |
| `issue fix <id\|description>` | Start fixer; `--rollout`, `--budget-usd`, `--model` |
| `issue settings` | Read/change opt-in automation, node/place/model/cadence/budgets, `--mode ask\|auto` (the fixers' guard), `--auto-land on\|off`, `--places` (a second repository built against this one); asks you when an agent runs it |
| `issue schedule` | Create issue-triage schedule from settings |
| `issue fetch` | Fetch origin into node mirror; print fixer fetch instructions |
| `issue push <id> [branches…]` | Push fixer's permitted branches through node |
| `issue ci <id> <commit>` | Inspect CI through node |
| `issue land <id> <commit> [--summary <text>]` | Fast-forward after CI and required protected-path approval |
| `vault create --endpoint <url>` | Experimental: initialize vault and print recovery kit once |
| `vault join --endpoint <url> --vault <id> [--kit] [--role <role>]` | Join or print admission keys |
| `vault admit <device> <seal> [--role <role>]` | Admit a device as vault admin |
| `vault status [--json]` | Standing, pending actions and last backup |
| `vault backup [--watch <secs>]` | Incremental record/blob backup |
| `vault recover --preview <new-dir>` | Inert recovery; `--staging`, `--through`, `--resume`, `--phone`, `--kit`, `--endpoint` |
| `vault import <preview> --id <id>` | Import selected records; repeat `--id`, optional `--root` |
| `vault disable` | Move aside local vault state, not delete remote vault |

Read `stitch discord --help` and `stitch proxy --help`, [self-improvement](self-improve.md), or
[the vault's protocol and limitations](vault-protocol.md) **before** enabling these. The vault
has no hosted production service. Do not treat it as a proven backup solution.

## Low-level records and maintenance

| Command | Purpose |
|---|---|
| `new <kind>` | Write immutable record; `--title`, `--body` (`-` for stdin), repeatable `--ref`, `--field`, `--attach`, `--user` |
| `show <id>` | Record and derived status |
| `ls [--kind <kind>]` | Records, oldest first |
| `edges` | Graph edges (`src rel dst`) |
| `sync-local <other-root>` | Reconcile record roots on this machine |
| `reindex` | Rebuild derived indexes from home/project records |
| `help [command…]` | Help for any command path |

Use `--root` to select a record root. See [records.md](records.md) for formats,
relations and status derivation. Reindexing is not restoring missing records.

## Terminal UI

`/` opens the filterable palette; `/help` prints help in terminal scrollback.
The UI keeps input and streaming output live while saved conversation text
remains in normal terminal scrollback.

| Slash command | Purpose |
|---|---|
| `/model [provider/model]` | Pick model; `/model default` follows default role |
| `/spawn [@node[:model]] <goal>` | Delegate from current conversation; alone opens node/model picker |
| `/send [thread] [message]` | Continue a thread and receive its report |
| `/threads` | Thread picker |
| `/parent` / `/children` | Open parent / pick an immediate child |
| `/rename <title>` / `/pin` | Title / toggle pin |
| `/quote` | New thread with this conversation as quoted material |
| `/search [--here] <words>` | Search and open a hit |
| `/jobs` | Background jobs |
| `/new [global]` | New thread in place, or no place |
| `/fork [N] [-a] [-t] [-s] [@m12]` | Fork; exclude attachments/tool calls, summarize omitted part, choose cutoff; alone opens form |
| `/summarize` | New continuation from summary |
| `/compact [N]` | Summary plus last N messages (10 by default); alone opens form |
| `/attach <file>` / `/drop` | Add / clear pending attachments |
| `/memory [on\|off]` | Inspect/change memory inclusion |
| `/run-on <node\|here>` | Run thread on another node, streaming here, or return here |
| `/read-only [on\|off]` | Inspect/change read-only mode |
| `/tools` / `/skills` | Toggle per-thread policy |
| `/link [--force] node:path` / `/unlink` | Folder identity links |
| `/copy` | Copy latest reply |
| `/app [name]` | Open latest thread app, or named pinned app |
| `/yolo [on\|off]` | Inspect/change shell approval bypass |
| `/login [provider] [--file]` | Subscription or GitHub sign-in picker |
| `/logout [provider]` / `/auth [provider]` | Sign out / inspect sign-in |
| `/help` / `/quit` | Print help / exit client |

### Keys

| Key | Action |
|---|---|
| Enter | Send, or queue/interject while agent works |
| Shift+Enter / Alt+Enter / Ctrl+J / `\` then Enter | Newline (terminal support varies) |
| Ctrl+O / Ctrl+N | Threads / new thread |
| Ctrl+P | Parent thread |
| Ctrl+T / Ctrl+B | Attention / jobs |
| Ctrl+Y | Copy latest reply |
| Ctrl+V | Attach clipboard image or copied file |
| `@` in input | Fuzzy file picker |
| Esc while working | Interrupt; unconsumed queued input comes back |
| Up while working | Retrieve latest queued message for editing |
| Esc Esc with empty input | Message picker: fork, edit/resend, copy |

In pickers, type to filter and Esc to close. In the thread picker, Tab cycles
here-and-global / here-only / all. In tools/skills, Enter toggles. In jobs, `x`
stops. In attention, inspect the question and answer with `y` or `n`; an `always`
grant is persistent and scoped, not just another spelling of yes.
