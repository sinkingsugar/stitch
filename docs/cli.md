# CLI reference

Generated from `stitch --help` by `tools/cli-reference.sh`; do not edit by hand.
Guides: [setup](setup.md) and [features](features.md).

## `stitch`

```text
Stitch: chats and agents on every machine you own

Usage: stitch [OPTIONS] [COMMAND]

Commands:
  setup         Name this node and create its home workspace
  node          Run the node in the foreground, or control the running one
  shell         The node's shell sessions
  chat          Send a message: a new thread, or one given with --thread
  scope         What attaching threads or efforts to a message tells the model (the briefing `--scope` writes with it), and with `--search`, what `recall` with that scope finds by keyword in every thread of it
  answer        Answer an approval a thread is waiting on (see `stitch attention`)
  cancel        Stop a running detached thread
  schedule      Schedules: Stitch waking on its own at times, continuing a detached thread at each firing (design §2.3)
  event         Events from outside programs (docs/events.md): publish one, list them, subscribe a thread or a schedule to a topic pattern
  rename        Name a thread (its title, on every node)
  pin           Pin a thread: first in lists, on every node (`--off` unpins)
  artefacts     What a conversation touched (paths read and written, URLs read, commits written, threads spawned and told), from its tool calls, the latest first; the threads of its compaction chain together
  digest        A conversation's digest: where it stands, what it decided, found and changed, what it left open and asks of you, each item with the message it comes from. `--update` runs one update now (a model call, priced; the day's cap holds) and says what it cost. With no thread (or `--status`): how digests stand on this node: conversations with a fresh digest, a stale one, none; the day's cost against the cap; the backfill sweep. `--sweep` runs one sweep now
  touched       Every conversation that touched a path, URL, commit or thread, in time order. The kind is guessed from the value (`--kind` says it)
  instructions  Your own instructions for the place you are in (this project or folder), or for every thread (`--global`): synced to every node, told to threads before the checkout's AGENTS.md. Shown, or edited
  search        Search every thread's messages: by their words, and by their meaning where memory runs (EmbeddingGemma 2); best first
  fork          Fork a thread: a new one continuing it from a message (read from it, not copied), with or without its attachments and tool calls, all of it or its last messages; `--summarize` puts a summary in place of what is not kept. Prints the new thread; continue it with `stitch chat -t <id>`
  jobs          What runs in the background: shell commands on this node (with the job that tells their thread when they end) and detached threads on every node. `stop <id>` stops one: Ctrl+C to a command (again: its session closed), a thread cancelled
  mcp           Check the MCP servers in config.toml: start each, list its tools (those threads here get marked), stop it again
  decide        Decisions (design §2.29): closed questions answered by the decision model with calibrated probabilities, for the guard, the landing gate, the triage, loose ends and agents' own (`decide` tool). Alone: the profiles, their thresholds and fallbacks, and the node's day
  yolo          Shell commands run without asking (`on`: `stitch mode yolo`), or ask again (`off`: `stitch mode ask`); alone, the mode. Saved in config.toml
  mode          The approval mode (design §2.28): `ask` (sandboxed where the system can, what reaches beyond asks; else what the rules do not allow asks), `auto` (yolo with the hazard guard: what could do severe damage that cannot be undone asks, and without the decision model's verdict a command does not run) or `yolo` (everything runs, unjudged); alone, the mode. Saved in config.toml. A place whose `.stitch/settings.toml` says `approval = "…"` follows that instead: run inside one, it says so
  send          Continue a thread with a message, on the node that owns it (a detached thread's follow-up, any thread on another machine)
  room          Rooms (design §2.27): a conversation with several participants, each a thread of its own (fresh models, read-only by default, or existing threads). Talk in one with `stitch chat --thread <room>` or `stitch send <room>`; `@handle` picks who answers
  issue         Harness issues: bugs in Stitch itself, filed by agents (`report_issue`) or you, and the fixes `self_improve` starts (docs/self-improve.md)
  notify        Send a notification to your phones now (what an agent's `notify` does): from a script, a schedule's check, a long build's end
  threads       List chat threads: this folder's (or project's), then global ones, most recent first
  thread        Print a thread's transcript (`last`: the latest here), switch it between read-only and writable, or show and change its tools and skills
  key           Provider API keys: secrets by the provider's name (`stitch secret`)
  secret        Secrets (design §2.19): kept in the keychain, synced sealed to your other nodes unless this device's only, given to shells by `[env]`, never shown to a model
  vault         The node's opt-in end-to-end encrypted vault: backup and store-and-forward (docs/vault-protocol.md; experimental, no hosted service yet)
  proxy         A key-holding proxy for model APIs: clients (a bot's node in a container) get tokens of their own, with budgets, and never the key
  discord       A Discord bot in front of this node (`DISCORD_TOKEN` or `DISCORD_TOKEN_FILE`): a mention or `/ask` in an allowed guild starts a thread, answered in a Discord thread
  var           Variables: values that are not secret, synced as records, given to shells by their names
  model         Models, ranked as agents see them, and your settings for them (reserved, tier, price, note), synced to every node
  ssh           SSH hosts (design §2.20): records every node knows, the shell tools' `host`; their host keys; this device's key and authorizing it
  auth          Subscription sign-in (Anthropic, OpenAI/Codex, Grok)
  apns          Push notifications to your phones, sent by this node through Apple's push service with your own key
  app           Apps agents made (design §2.10): pinned ones, opened in the browser
  dashboard     The dashboard: your threads on every node, what needs you, jobs, live; pinned (with the phone too) and opened in the browser
  client        Open the web client (the node's own UI for a browser: conversations, home, approvals) and print its address
  loose-ends    Loose ends: the conversations that wait on you. Judges first (the decision model, the quick model where it is unsure, on those not judged at their latest step), then lists; the node does it hourly by itself (`[loose_ends]`)
  invite        Show a pairing ticket (QR code and text) for a new node
  join          Join the user's nodes with a ticket from `stitch invite`
  sync          Sync with every known node now, or with one
  id            Print this node's endpoint ID
  peer          The nodes trusted by this one
  project       Projects: every git checkout is one, known by its first commit
  browser       The user's own Chrome through Stitch's extension, and the headless Chromium the web tools drive
  memory        Memory: what was said and seen, recalled across conversations
  folder        Folders outside git: the same `~` path is the same folder on every node; these join or part others
  new           Write a new record and print its ID
  show          Print a record and its derived status
  ls            List records, oldest first
  attention     What is blocked on the user
  edges         Every edge of the graph, as `src rel dst`
  sync-local    Reconcile the root with another record root on this machine
  reindex       Drop the index and rebuild it from the home workspace and the projects
  help          Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
  -V, --version      Print version
```

### `stitch setup`

```text
Name this node and create its home workspace

Usage: stitch setup [OPTIONS] --name <NAME>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --name <NAME>
      --home <HOME>  Where the home workspace lives (default: `home` in the data directory)
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --no-home      Do not create a home workspace (a node that will join another's)
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
      --local        Direct addresses only: no relays, nothing published to discovery
  -h, --help         Print help
```

### `stitch node`

```text
Run the node in the foreground, or control the running one

Usage: stitch node [OPTIONS] [COMMAND]

Commands:
  stop     Stop the running node (its shells and running turns end; detached threads resume at its next start)
  restart  Restart the node once nothing runs (or `--now`): the node starts its successor itself, from the binary installed now, outside any sandbox of the shell asking. Detached threads resume
  status   The running node's name, ID, build, repositories and peers
  builds   The builds this node keeps and their health (`<data>/bin`): the healthy one and since when, the one on trial, those that failed
  service  The node as a service: a systemd user unit (Linux) or a LaunchAgent (macOS), started at login, started again when it asks to restart
  help     Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch node stop`

```text
Stop the running node (its shells and running turns end; detached threads resume at its next start)

Usage: stitch node stop [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --when-idle    Once nothing runs instead of now (a service then takes over)
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch node restart`

```text
Restart the node once nothing runs (or `--now`): the node starts its successor itself, from the binary installed now, outside any sandbox of the shell asking. Detached threads resume

Usage: stitch node restart [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --now          Now: running turns are cut and resume, shells end
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch node status`

```text
The running node's name, ID, build, repositories and peers

Usage: stitch node status [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch node builds`

```text
The builds this node keeps and their health (`<data>/bin`): the healthy one and since when, the one on trial, those that failed

Usage: stitch node builds [OPTIONS]

Options:
      --data <DATA>         The node's data directory [env: STITCH_DATA]
      --healthy-for <SECS>  Succeed only when the build installed is the one running, healthy for at least this many seconds (a rollout's trial)
      --root <ROOT>         The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>         Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help                Print help
```

#### `stitch node service`

```text
The node as a service: a systemd user unit (Linux) or a LaunchAgent (macOS), started at login, started again when it asks to restart

Usage: stitch node service [OPTIONS] <COMMAND>

Commands:
  install    Write, enable and start it. A node running outside it stops once idle, and the service's takes over
  uninstall  Stop and remove it (and the update timer)
  help       Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

##### `stitch node service install`

```text
Write, enable and start it. A node running outside it stops once idle, and the service's takes over

Usage: stitch node service install [OPTIONS]

Options:
      --data <DATA>                  The node's data directory [env: STITCH_DATA]
      --update [<CHECKOUT>]          Also update from main every 30 minutes (pull, build, install; the node restarts into the new build once idle), from this checkout
      --follow <FOLLOW>              The rollout (with --update): follow this branch instead of main (`fleet`: what the canary promoted)
      --root <ROOT>                  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>                  Overrides the node name from config.toml [env: STITCH_NODE]
      --promote <PROMOTE>            The rollout: push a build healthy here for 30 minutes to this branch, for the next nodes (the canary: `fleet`)
      --testflight [<APP_CHECKOUT>]  The rollout, macOS: upload the app to TestFlight once a new build is healthy here (the phone follows), with the app checkout's justfile (default: `--app`'s, else the checkout's own when it holds the app)
      --app <APP_CHECKOUT>           The rollout: the app's checkout, followed and promoted with this one (the same branch), so a commit landed in the app's repository alone reaches the Mac that builds it (docs/self-improve.md §6)
      --exe <EXE>                    The binary it runs (default: this one)
      --print                        Only print the files it would write
  -h, --help                         Print help
```

##### `stitch node service uninstall`

```text
Stop and remove it (and the update timer)

Usage: stitch node service uninstall [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch shell`

```text
The node's shell sessions

Usage: stitch shell [OPTIONS] <COMMAND>

Commands:
  attach  Put a session an agent drives in this terminal, live (`<thread>/<session>` as `stitch jobs` lists it): watch it, type into it (the agent is told, and sends nothing on top of it until it looked). Ctrl+] detaches
  help    Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch shell attach`

```text
Put a session an agent drives in this terminal, live (`<thread>/<session>` as `stitch jobs` lists it): watch it, type into it (the agent is told, and sends nothing on top of it until it looked). Ctrl+] detaches

Usage: stitch shell attach [OPTIONS] <ID>

Arguments:
  <ID>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch chat`

```text
Send a message: a new thread, or one given with --thread

Usage: stitch chat [OPTIONS] [TEXT]

Arguments:
  [TEXT]  The message; `-` or nothing reads it from standard input

Options:
      --data <DATA>                    The node's data directory [env: STITCH_DATA]
  -t, --thread <THREAD>                A thread ID, or `last` (the latest here, else the latest global one)
      --global                         A new thread that belongs to no folder
      --root <ROOT>                    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
  -a, --attach <ATTACHMENTS>           A file to attach, repeatable
      --node <NODE>                    Overrides the node name from config.toml [env: STITCH_NODE]
  -m, --model <MODEL>                  `provider/model`, e.g. `anthropic/claude-opus-5-5`
      --repo <REPO>                    The repository a new thread lives in (default: the project whose checkout this is, else the home workspace)
  -q, --quiet                          Do not show the model's reasoning summaries
      --detach                         Run it on its own (a detached thread toward this goal); its report goes to --parent, or to the thread whose shell runs this command
      --parent <PARENT>                [env: STITCH_THREAD]
      --budget-usd <BUDGET_USD>        Budget for a detached thread: dollars
      --budget-tokens <BUDGET_TOKENS>  Budget for a detached thread: tokens
      --minutes <MINUTES>              Budget for a detached thread: minutes
      --max-steps <MAX_STEPS>          Model calls each turn of the detached thread takes at most (default: the node's `[detached] max_steps`, else 50)
      --on <ON>                        Run the detached thread on another of the user's nodes (delegation)
      --context [<CONTEXT>]            The conversation the detached thread starts from: `this` (--parent's so far, the default when given bare), `none`, or a thread's ID
      --no-tools                       The detached thread runs without tools (a question to a model)
      --no-wake                        Its report is kept for the user on --parent's thread (shown when it is opened) and never wakes it
      --memory                         Memory remembers the detached thread (by default it is out of memory from its start: nothing of it is remembered, though it may recall what other conversations left)
      --read-only                      The detached thread is read-only: commands and calls the rules do not allow are refused, never asked, so it never waits on you
      --tools <TOOLS>                  Changes to the thread's tools, comma-separated: `+github` (an MCP server), `-web` (a built-in group), `+github__get_*` (a pattern)
      --skills <SKILLS>                Changes to the thread's skills, comma-separated: `-release`
      --scope <SCOPES>                 A thread or effort to attach to the message, repeatable: `effort:<id>` (a conversation and every thread spawned under it), `thread:<id>` or a thread's ID. The model is briefed on it (design §2.27); `stitch scope` shows what it would be told. With `--detach`, attached to its goal (as the `spawn` tool's `scopes`)
  -h, --help                           Print help
```

### `stitch scope`

```text
What attaching threads or efforts to a message tells the model (the briefing `--scope` writes with it), and with `--search`, what `recall` with that scope finds by keyword in every thread of it

Usage: stitch scope [OPTIONS] <TARGETS>...

Arguments:
  <TARGETS>...  `effort:<id>`, `thread:<id>` or a thread's ID; several together

Options:
      --data <DATA>      The node's data directory [env: STITCH_DATA]
      --search <SEARCH>
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help             Print help
```

### `stitch answer`

```text
Answer an approval a thread is waiting on (see `stitch attention`)

Usage: stitch answer [OPTIONS] <QUESTION> <DECISION> [REASON]...

Arguments:
  <QUESTION>
  <DECISION>   `y` allows, `n` declines, `always` allows and lets what the question grants (shown by `stitch attention`) run without asking from then on where its thread works
  [REASON]...  Why, when declining

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch cancel`

```text
Stop a running detached thread

Usage: stitch cancel [OPTIONS] <THREAD>

Arguments:
  <THREAD>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch schedule`

```text
Schedules: Stitch waking on its own at times, continuing a detached thread at each firing (design §2.3)

Usage: stitch schedule [OPTIONS] <COMMAND>

Commands:
  add     Make one, in the place you are in (or `--place`), pinned to this node (or `--on <node>`): `stitch schedule add --every "0 9 * * 1-5" "Triage yesterday's CI failures"`
  list    The schedules (`--all`: the ended ones too)
  show    One schedule, with its firings and what each cost
  next    The next firings
  pause
  resume
  run     Fire it now (on the node it is pinned to)
  rm      End it
  help    Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch schedule add`

```text
Make one, in the place you are in (or `--place`), pinned to this node (or `--on <node>`): `stitch schedule add --every "0 9 * * 1-5" "Triage yesterday's CI failures"`

Usage: stitch schedule add [OPTIONS] <GOAL>...

Arguments:
  <GOAL>...  What each firing asks its thread to do

Options:
      --data <DATA>
          The node's data directory [env: STITCH_DATA]
      --name <NAME>

      --at <AT>
          Once: RFC 3339, `YYYY-MM-DD HH:MM` (in `--tz`), or `in 2h`
      --root <ROOT>
          The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --every <EVERY>
          Recurring: a five-field cron expression, `@daily` and the like, or `@every 30m`
      --node <NODE>
          Overrides the node name from config.toml [env: STITCH_NODE]
      --tz <TZ>
          The IANA zone `at` and `every` are read in (default: the node's)
      --place <PLACE>
          `global`, `project:<repo>` or `folder:<folder>`
      --on <ON>
          The node that fires it (`--node` names this one, as everywhere)
      --model <MODEL>

      --max-steps <MAX_STEPS>

      --read-only

      --budget-usd <BUDGET_USD>
          Each firing's budget
      --budget-tokens <BUDGET_TOKENS>

      --budget-minutes <BUDGET_MINUTES>

      --daily-cap-usd <DAILY_CAP_USD>
          At most this much a day over all its firings
      --check <CHECK>
          Otter code run first, at no model cost: nothing returned, nothing woken
      --check-file <CHECK_FILE>
          Instead of `--check`: a `.otr` file of the place
      --fresh
          A new thread at each firing, not one continued
      --no-catch-up
          Skip a firing missed while the node was down (default: run it once)
      --reports-to <REPORTS_TO>
          The thread its reports go to
      --wake
          Its reports wake that thread (default: kept for you)
      --on-event <ON_EVENT>
          An event trigger (docs/events.md): events on topics matching this pattern fire it (`ci/failure/*`), its check run on each with the payload as input; without `--at` or `--every`, events alone
      --debounce <DEBOUNCE>
          With `--on-event`: events within this many seconds wake once
      --max-per-hour <MAX_PER_HOUR>
          With `--on-event`: wakes per hour at most
      --until <UNTIL>
          With `--on-event`: when the trigger ends
      --tools <TOOLS>
          Allowed tools, comma-separated: groups (`viewer`), names (`notify`), patterns (`support_*`); `event_reply` is always allowed
      --shards <SHARDS>
          Otter shard entries, comma-separated, the last matching deciding: `+Http.Get`, `-FS.*`
      --effects <EFFECTS>
          Untrusted: effect classes a shard may have without being named (`io,time,suspends,random,unknown`; default none: pure shards only)
      --untrusted
          A profile with nothing but `event_reply` (plus what `--tools` adds)
      --trusted
          The events are your own programs': no taint, and `--shards` only denies
  -h, --help
          Print help
```

#### `stitch schedule list`

```text
The schedules (`--all`: the ended ones too)

Usage: stitch schedule list [OPTIONS]

Options:
      --all
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch schedule show`

```text
One schedule, with its firings and what each cost

Usage: stitch schedule show [OPTIONS] <SCHEDULE>

Arguments:
  <SCHEDULE>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch schedule next`

```text
The next firings

Usage: stitch schedule next [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch schedule pause`

```text
Usage: stitch schedule pause [OPTIONS] <SCHEDULE>

Arguments:
  <SCHEDULE>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch schedule resume`

```text
Usage: stitch schedule resume [OPTIONS] <SCHEDULE>

Arguments:
  <SCHEDULE>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch schedule run`

```text
Fire it now (on the node it is pinned to)

Usage: stitch schedule run [OPTIONS] <SCHEDULE>

Arguments:
  <SCHEDULE>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch schedule rm`

```text
End it

Usage: stitch schedule rm [OPTIONS] <SCHEDULE>

Arguments:
  <SCHEDULE>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch event`

```text
Events from outside programs (docs/events.md): publish one, list them, subscribe a thread or a schedule to a topic pattern

Usage: stitch event [OPTIONS] <COMMAND>

Commands:
  publish      Publish an event: `stitch event publish ci/failure/test '{"job": "test-linux"}'`. With `--ask <secs>`, wait for a thread's answer (printed as JSON) until that deadline; exit 2 when none came
  status       An event's state (an ask's answer), waiting up to `--wait` seconds for an ask to be answered
  ls           The subscriptions, and the events kept here (newest first)
  subscribe    Subscribe a thread (woken with the events, within its own budget) or a schedule (fired by them) to a topic pattern (`*` one segment, `**` any)
  unsubscribe  End a subscription (a schedule's event trigger comes off it)
  untaint      Lift the taint untrusted events left on a thread (its capability profiles): its next turn has its own tools again
  help         Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch event publish`

```text
Publish an event: `stitch event publish ci/failure/test '{"job": "test-linux"}'`. With `--ask <secs>`, wait for a thread's answer (printed as JSON) until that deadline; exit 2 when none came

Usage: stitch event publish [OPTIONS] <TOPIC> [PAYLOAD]

Arguments:
  <TOPIC>
  [PAYLOAD]  The payload: JSON (`-` reads it from stdin; none: null)

Options:
      --ask <ASK>        An ask: the deadline in seconds
      --data <DATA>      The node's data directory [env: STITCH_DATA]
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --schema <SCHEMA>  With `--ask`: the JSON Schema the answer must match
      --no-wait          With `--ask`: print the ask's id and return at once (`stitch event status` follows it)
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help             Print help
```

#### `stitch event status`

```text
An event's state (an ask's answer), waiting up to `--wait` seconds for an ask to be answered

Usage: stitch event status [OPTIONS] <ID>

Arguments:
  <ID>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --wait <WAIT>  [default: 0]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch event ls`

```text
The subscriptions, and the events kept here (newest first)

Usage: stitch event ls [OPTIONS]

Options:
      --data <DATA>    The node's data directory [env: STITCH_DATA]
      --topic <TOPIC>  Only topics matching this pattern
      --limit <LIMIT>  [default: 20]
      --root <ROOT>    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>    Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help           Print help
```

#### `stitch event subscribe`

```text
Subscribe a thread (woken with the events, within its own budget) or a schedule (fired by them) to a topic pattern (`*` one segment, `**` any)

Usage: stitch event subscribe [OPTIONS] <PATTERN>

Arguments:
  <PATTERN>

Options:
      --data <DATA>                  The node's data directory [env: STITCH_DATA]
      --thread <THREAD>
      --root <ROOT>                  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --schedule <SCHEDULE>
      --filter <FILTER>              Otter code run on each event (its payload the input), at no model cost: nothing returned drops it
      --node <NODE>                  Overrides the node name from config.toml [env: STITCH_NODE]
      --filter-file <FILTER_FILE>    Instead of `--filter`: a `.otr` file in the thread's directory
      --debounce <DEBOUNCE>          Events within this many seconds wake once (default 2)
      --max-per-hour <MAX_PER_HOUR>  Wakes per hour at most
      --daily-usd <DAILY_USD>        What its wakes may spend a day
      --until <UNTIL>                When it ends: RFC 3339, `YYYY-MM-DD HH:MM`, or `in 2h`
      --note <NOTE>                  What each wake tells the thread
      --tools <TOOLS>                Allowed tools, comma-separated: groups (`viewer`), names (`notify`), patterns (`support_*`); `event_reply` is always allowed
      --shards <SHARDS>              Otter shard entries, comma-separated, the last matching deciding: `+Http.Get`, `-FS.*`
      --effects <EFFECTS>            Untrusted: effect classes a shard may have without being named (`io,time,suspends,random,unknown`; default none: pure shards only)
      --untrusted                    A profile with nothing but `event_reply` (plus what `--tools` adds)
      --trusted                      The events are your own programs': no taint, and `--shards` only denies
      --answers                      With `--thread`: the thread answers the asks this subscription delivers (`event_reply`) even when it is read-only
  -h, --help                         Print help
```

#### `stitch event unsubscribe`

```text
End a subscription (a schedule's event trigger comes off it)

Usage: stitch event unsubscribe [OPTIONS] <SUBSCRIPTION>

Arguments:
  <SUBSCRIPTION>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch event untaint`

```text
Lift the taint untrusted events left on a thread (its capability profiles): its next turn has its own tools again

Usage: stitch event untaint [OPTIONS] <THREAD>

Arguments:
  <THREAD>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch rename`

```text
Name a thread (its title, on every node)

Usage: stitch rename [OPTIONS] <THREAD> <TITLE>...

Arguments:
  <THREAD>
  <TITLE>...

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch pin`

```text
Pin a thread: first in lists, on every node (`--off` unpins)

Usage: stitch pin [OPTIONS] <THREAD>

Arguments:
  <THREAD>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --off
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch artefacts`

```text
What a conversation touched (paths read and written, URLs read, commits written, threads spawned and told), from its tool calls, the latest first; the threads of its compaction chain together

Usage: stitch artefacts [OPTIONS] <THREAD>

Arguments:
  <THREAD>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch digest`

```text
A conversation's digest: where it stands, what it decided, found and changed, what it left open and asks of you, each item with the message it comes from. `--update` runs one update now (a model call, priced; the day's cap holds) and says what it cost. With no thread (or `--status`): how digests stand on this node: conversations with a fresh digest, a stale one, none; the day's cost against the cap; the backfill sweep. `--sweep` runs one sweep now

Usage: stitch digest [OPTIONS] [THREAD]

Arguments:
  [THREAD]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --update
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --status
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
      --sweep
  -h, --help         Print help
```

### `stitch touched`

```text
Every conversation that touched a path, URL, commit or thread, in time order. The kind is guessed from the value (`--kind` says it)

Usage: stitch touched [OPTIONS] <VALUE>

Arguments:
  <VALUE>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --kind <KIND>  `path`, `url`, `commit` or `thread`
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch instructions`

```text
Your own instructions for the place you are in (this project or folder), or for every thread (`--global`): synced to every node, told to threads before the checkout's AGENTS.md. Shown, or edited

Usage: stitch instructions [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --global       Every thread's, not this place's
      --edit         Edit them in $EDITOR
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
      --set <SET>    Set them from a file (`-`: standard input; empty: none)
  -h, --help         Print help
```

### `stitch search`

```text
Search every thread's messages: by their words, and by their meaning where memory runs (EmbeddingGemma 2); best first

Usage: stitch search [OPTIONS] <QUERY>...

Arguments:
  <QUERY>...  What to look for

Options:
      --data <DATA>    The node's data directory [env: STITCH_DATA]
      --here           Only the threads of the place you are in (this project or folder)
  -n, --limit <LIMIT>  How many to show [default: 20]
      --root <ROOT>    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>    Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help           Print help
```

### `stitch fork`

```text
Fork a thread: a new one continuing it from a message (read from it, not copied), with or without its attachments and tool calls, all of it or its last messages; `--summarize` puts a summary in place of what is not kept. Prints the new thread; continue it with `stitch chat -t <id>`

Usage: stitch fork [OPTIONS] <THREAD>

Arguments:
  <THREAD>  The thread, or `last` (the latest here)

Options:
      --data <DATA>     The node's data directory [env: STITCH_DATA]
      --upto <UPTO>     Up to this message (`m12`, as `stitch thread` shows them)
      --last <LAST>     Only its last N messages (from one of yours)
      --root <ROOT>     The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --no-attachments
      --node <NODE>     Overrides the node name from config.toml [env: STITCH_NODE]
      --no-tools        What was said only: no tool calls or results
      --summarize       A summary stands for what is not kept (all of it, or what comes before `--last`)
  -h, --help            Print help
```

### `stitch jobs`

```text
What runs in the background: shell commands on this node (with the job that tells their thread when they end) and detached threads on every node. `stop <id>` stops one: Ctrl+C to a command (again: its session closed), a thread cancelled

Usage: stitch jobs [OPTIONS] [ACTION] [ID]

Arguments:
  [ACTION]  [possible values: stop]
  [ID]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch mcp`

```text
Check the MCP servers in config.toml: start each, list its tools (those threads here get marked), stop it again

Usage: stitch mcp [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch decide`

```text
Decisions (design §2.29): closed questions answered by the decision model with calibrated probabilities, for the guard, the landing gate, the triage, loose ends and agents' own (`decide` tool). Alone: the profiles, their thresholds and fallbacks, and the node's day

Usage: stitch decide [OPTIONS] [COMMAND]

Commands:
  profiles  Every profile: its questions, thresholds (yours over the defaults), fallback, model; the node's day
  set       Change a profile's thresholds or fallback, for every node: `guard.user --threshold 0.6` (its one question, or one for every question) or `--threshold key=0.6`. Without a profile: `--model m`, the decision model, and `--fallback <model|none>`, the model that answers (uncalibrated) when the decision model cannot (none by default: the call fails; never for guard.fixer and land, which ask)
  try       Ask a profile's questions of a text (`-`: stdin) and print the answers; `--yes-no "…?"` (repeatable) asks your own instead (the `tool` profile)
  log       The latest decisions this node recorded (newest last)
  help      Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch decide profiles`

```text
Every profile: its questions, thresholds (yours over the defaults), fallback, model; the node's day

Usage: stitch decide profiles [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch decide set`

```text
Change a profile's thresholds or fallback, for every node: `guard.user --threshold 0.6` (its one question, or one for every question) or `--threshold key=0.6`. Without a profile: `--model m`, the decision model, and `--fallback <model|none>`, the model that answers (uncalibrated) when the decision model cannot (none by default: the call fails; never for guard.fixer and land, which ask)

Usage: stitch decide set [OPTIONS] [PROFILE]

Arguments:
  [PROFILE]  A profile's name (`stitch decide profiles`); none for every profile's model and fallback

Options:
      --data <DATA>             The node's data directory [env: STITCH_DATA]
      --threshold <THRESHOLDS>  `0.6` or `key=0.6` (a choice's option: `verdict:done=0.9`); repeatable
      --fallback <FALLBACK>     The fallback model (`provider/model`, `quick`) or `none`
      --root <ROOT>             The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --model <MODEL>           The decision model, `provider/model` (without a profile)
      --node <NODE>             Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help                    Print help
```

#### `stitch decide try`

```text
Ask a profile's questions of a text (`-`: stdin) and print the answers; `--yes-no "…?"` (repeatable) asks your own instead (the `tool` profile)

Usage: stitch decide try [OPTIONS] <PROFILE> <INPUT>

Arguments:
  <PROFILE>  A profile's name (`tool` for your own questions)
  <INPUT>    The text, or `-` for stdin

Options:
      --data <DATA>      The node's data directory [env: STITCH_DATA]
      --yes-no <YES_NO>  A yes/no question of your own (repeatable), instead of the profile's
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help             Print help
```

#### `stitch decide log`

```text
The latest decisions this node recorded (newest last)

Usage: stitch decide log [OPTIONS]

Options:
      --data <DATA>        The node's data directory [env: STITCH_DATA]
      --profile <PROFILE>  Only this profile's
  -n <N>                   How many, the latest [default: 20]
      --root <ROOT>        The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>        Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help               Print help
```

### `stitch yolo`

```text
Shell commands run without asking (`on`: `stitch mode yolo`), or ask again (`off`: `stitch mode ask`); alone, the mode. Saved in config.toml

Usage: stitch yolo [OPTIONS] [STATE]

Arguments:
  [STATE]  [possible values: on, off]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch mode`

```text
The approval mode (design §2.28): `ask` (sandboxed where the system can, what reaches beyond asks; else what the rules do not allow asks), `auto` (yolo with the hazard guard: what could do severe damage that cannot be undone asks, and without the decision model's verdict a command does not run) or `yolo` (everything runs, unjudged); alone, the mode. Saved in config.toml. A place whose `.stitch/settings.toml` says `approval = "…"` follows that instead: run inside one, it says so

Usage: stitch mode [OPTIONS] [MODE]

Arguments:
  [MODE]  [possible values: ask, auto, yolo]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch send`

```text
Continue a thread with a message, on the node that owns it (a detached thread's follow-up, any thread on another machine)

Usage: stitch send [OPTIONS] <THREAD> [TEXT]

Arguments:
  <THREAD>
  [TEXT]    The message; standard input when omitted or `-`

Options:
      --data <DATA>     The node's data directory [env: STITCH_DATA]
      --from <FROM>     The thread its reply is reported to (and that is woken with it) [env: STITCH_THREAD]
      --no-wake         The reply is kept for the user on --from's thread and never wakes it
      --root <ROOT>     The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --interject       Weave the message into the turn running on the thread, between its steps, instead of waiting for it to end; its last reply is the reply. With no turn running it starts one
      --node <NODE>     Overrides the node name from config.toml [env: STITCH_NODE]
      --scope <SCOPES>  A thread or effort to attach to the message, repeatable (as for `chat`)
  -h, --help            Print help
```

### `stitch room`

```text
Rooms (design §2.27): a conversation with several participants, each a thread of its own (fresh models, read-only by default, or existing threads). Talk in one with `stitch chat --thread <room>` or `stitch send <room>`; `@handle` picks who answers

Usage: stitch room [OPTIONS] <COMMAND>

Commands:
  new   A new room with these participants: models (`provider/model`, `default`, `quick`, `model@node` to run it there) or threads (`thread:<id>`)
  pull  "Pull in model…": a room pulled out of a 1:1 thread, which joins it untouched, with these models briefed on it
  show  A room's participants and policy
  set   Changes a room
  help  Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch room new`

```text
A new room with these participants: models (`provider/model`, `default`, `quick`, `model@node` to run it there) or threads (`thread:<id>`)

Usage: stitch room new [OPTIONS] <PARTICIPANTS>...

Arguments:
  <PARTICIPANTS>...

Options:
      --data <DATA>              The node's data directory [env: STITCH_DATA]
      --title <TITLE>
      --place <PLACE>            Its place (`global`, `project:<repo>`, `folder:<folder>`)
      --root <ROOT>              The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>              Overrides the node name from config.toml [env: STITCH_NODE]
      --scope <SCOPES>           Threads or efforts its participants are briefed on (as `--scope`)
      --mode <MODE>              `sequential` (the default) or `parallel` (council)
      --hop-cap <HOP_CAP>
      --budget-usd <BUDGET_USD>  Dollars its participants may spend on its turns, together
      --writable                 Models added may write (not read-only)
      --no-tools                 Models added have no tools
  -h, --help                     Print help
```

#### `stitch room pull`

```text
"Pull in model…": a room pulled out of a 1:1 thread, which joins it untouched, with these models briefed on it

Usage: stitch room pull [OPTIONS] <THREAD> <MODELS>...

Arguments:
  <THREAD>
  <MODELS>...

Options:
      --data <DATA>              The node's data directory [env: STITCH_DATA]
      --mode <MODE>              `sequential` (the default) or `parallel` (council)
      --hop-cap <HOP_CAP>
      --root <ROOT>              The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --budget-usd <BUDGET_USD>  Dollars its participants may spend on its turns, together
      --node <NODE>              Overrides the node name from config.toml [env: STITCH_NODE]
      --writable                 Models added may write (not read-only)
      --no-tools                 Models added have no tools
  -h, --help                     Print help
```

#### `stitch room show`

```text
A room's participants and policy

Usage: stitch room show [OPTIONS] <ROOM>

Arguments:
  <ROOM>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch room set`

```text
Changes a room

Usage: stitch room set [OPTIONS] <ROOM>

Arguments:
  <ROOM>

Options:
      --add <ADD>          Participants to add (as for `new`), repeatable
      --data <DATA>        The node's data directory [env: STITCH_DATA]
      --remove <REMOVE>    Participants to remove, by handle
      --root <ROOT>        The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --mute <MUTE>
      --node <NODE>        Overrides the node name from config.toml [env: STITCH_NODE]
      --unmute <UNMUTE>
      --mode <MODE>        `sequential` or `parallel` (council)
      --hop-cap <HOP_CAP>  How many `@handoff`s between participants a message allows
      --writable           Models added may write (not read-only)
      --no-tools           Models added have no tools
  -h, --help               Print help
```

### `stitch issue`

```text
Harness issues: bugs in Stitch itself, filed by agents (`report_issue`) or you, and the fixes `self_improve` starts (docs/self-improve.md)

Usage: stitch issue [OPTIONS] <COMMAND>

Commands:
  list      The open issues and those being fixed, newest first
  show      One issue: what happened, where, its state and fix
  report    File one
  set       Change its state (open, fixing, fixed, dropped), note, fix or repository
  land      A fixer's: the node lands its commit on main for it (a fast-forward, once CI passed on that commit; touching a protected path, once the user approved what the node shows them of the diff), and sets the issue fixed
  push      A fixer's: the node pushes its branches for it (it reads no credentials): a bundle of them is made here (`git bundle create`, in this checkout), and the node pushes those that are the issue's own (`fix-<the issue ID's last part>`, alone or `-<name>`) from its mirror of origin, forced
  ci        CI's runs on a commit (or a branch as pushed), as the node reads them
  fetch     Fetches origin into the node's mirrors (one per repository), and says how to fetch from them (a fixer reads no credentials)
  revert    Revert an issue's landed fix on main (the node does it in its mirror, as a fast-forward) and open the issue again: yours; asks you when an agent runs it
  fix       Start a fixer for it on the fixer node (`self_improve`)
  schedule  Create the harness-issues schedule from the settings, pinned to the fixer node
  settings  Self-improvement's settings, synced to every node: show them, or change them (the fixer node, budgets, on or off). Asks when an agent runs it
  help      Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch issue list`

```text
The open issues and those being fixed, newest first

Usage: stitch issue list [OPTIONS]

Options:
      --all          Fixed and dropped ones too
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch issue show`

```text
One issue: what happened, where, its state and fix

Usage: stitch issue show [OPTIONS] <ISSUE>

Arguments:
  <ISSUE>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch issue report`

```text
File one

Usage: stitch issue report [OPTIONS] --what <WHAT> <TITLE>

Arguments:
  <TITLE>  What is wrong, in a line

Options:
      --data <DATA>          The node's data directory [env: STITCH_DATA]
      --what <WHAT>          What happened
      --expected <EXPECTED>  What should have
      --root <ROOT>          The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --evidence <EVIDENCE>
      --node <NODE>          Overrides the node name from config.toml [env: STITCH_NODE]
      --area <AREA>
  -h, --help                 Print help
```

#### `stitch issue set`

```text
Change its state (open, fixing, fixed, dropped), note, fix or repository

Usage: stitch issue set [OPTIONS] <ISSUE> [STATE]

Arguments:
  <ISSUE>
  [STATE]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --note <NOTE>
      --fix <FIX>    The commit that fixed it
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
      --repo <REPO>  The repository it is fixed in (`stitch issue settings` lists them)
  -h, --help         Print help
```

#### `stitch issue land`

```text
A fixer's: the node lands its commit on main for it (a fast-forward, once CI passed on that commit; touching a protected path, once the user approved what the node shows them of the diff), and sets the issue fixed

Usage: stitch issue land [OPTIONS] <ISSUE> <COMMIT>

Arguments:
  <ISSUE>
  <COMMIT>

Options:
      --data <DATA>        The node's data directory [env: STITCH_DATA]
      --summary <SUMMARY>  What the fix changes, in 1-3 lines: shown to the user as the fixer's words, beside the node's own facts
      --repo <REPO>        The repository (default: the commit's, else the one this directory is a checkout or worktree of)
      --root <ROOT>        The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>        Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help               Print help
```

#### `stitch issue push`

```text
A fixer's: the node pushes its branches for it (it reads no credentials): a bundle of them is made here (`git bundle create`, in this checkout), and the node pushes those that are the issue's own (`fix-<the issue ID's last part>`, alone or `-<name>`) from its mirror of origin, forced

Usage: stitch issue push [OPTIONS] <ISSUE> [BRANCHES]...

Arguments:
  <ISSUE>
  [BRANCHES]...  The branches (default: the one checked out)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --repo <REPO>  The repository (default: the one this directory is a checkout or worktree of)
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch issue ci`

```text
CI's runs on a commit (or a branch as pushed), as the node reads them

Usage: stitch issue ci [OPTIONS] <ISSUE> <COMMIT>

Arguments:
  <ISSUE>
  <COMMIT>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --repo <REPO>  The repository (default: the commit's, else this directory's)
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch issue fetch`

```text
Fetches origin into the node's mirrors (one per repository), and says how to fetch from them (a fixer reads no credentials)

Usage: stitch issue fetch [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --repo <REPO>  Only this repository's
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch issue revert`

```text
Revert an issue's landed fix on main (the node does it in its mirror, as a fast-forward) and open the issue again: yours; asks you when an agent runs it

Usage: stitch issue revert [OPTIONS] <ISSUE>

Arguments:
  <ISSUE>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch issue fix`

```text
Start a fixer for it on the fixer node (`self_improve`)

Usage: stitch issue fix [OPTIONS] <ISSUE>

Arguments:
  <ISSUE>  An issue's ID, or a description (filed first)

Options:
      --data <DATA>              The node's data directory [env: STITCH_DATA]
      --rollout <ROLLOUT>        none, canary or fleet (default)
      --budget-usd <BUDGET_USD>
      --root <ROOT>              The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --model <MODEL>
      --node <NODE>              Overrides the node name from config.toml [env: STITCH_NODE]
      --repo <REPO>              The repository it is fixed in (default: the issue's, else decided from it)
  -h, --help                     Print help
```

#### `stitch issue schedule`

```text
Create the harness-issues schedule from the settings, pinned to the fixer node

Usage: stitch issue schedule [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch issue settings`

```text
Self-improvement's settings, synced to every node: show them, or change them (the fixer node, budgets, on or off). Asks when an agent runs it

Usage: stitch issue settings [OPTIONS]

Options:
      --data <DATA>            The node's data directory [env: STITCH_DATA]
      --node <NODE>            The fixer node: where fixes run
      --place <PLACE>          The Stitch checkout there (default ~/devel/stitch)
      --root <ROOT>            The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --budget <BUDGET>        Each fix's budget, in dollars
      --daily <DAILY>          All fixes (and their triage) in a day, UTC, in dollars
      --every <EVERY>          The harness-issues schedule's cadence (cron)
      --model <MODEL>          The fixers' model (`provider/model`)
      --on
      --off
      --mode <MODE>            Fixers' and the triage's approvals: `ask` (the default) or `auto` (the hazard guard lets what it clears run in their sandbox; without its verdict they ask, never fail: design §2.28) [possible values: ask, auto]
      --auto-land <AUTO_LAND>  `on`: a protected fix the landing guard clears lands unasked, with a notice and the command that reverts it (default off) [possible values: on, off]
      --places <TOML>          The repositories fixes go to, when more than the core (docs/self-improve.md §6): a TOML array of tables, `[{ name = "core", path = "~/src/core" }, { name = "app", path = "~/src/app", core = "core", pin = "core.pin", areas = ["ios"] }]`; `""`: the core alone
  -h, --help                   Print help
```

### `stitch notify`

```text
Send a notification to your phones now (what an agent's `notify` does): from a script, a schedule's check, a long build's end

Usage: stitch notify [OPTIONS] [BODY]

Arguments:
  [BODY]  What it says; standard input when omitted or `-`

Options:
      --data <DATA>      The node's data directory [env: STITCH_DATA]
  -t, --title <TITLE>    A few words above it (default: Stitch)
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --thread <THREAD>  The thread it is about: opening it opens that chat (default: the thread whose shell runs this command) [env: STITCH_THREAD]
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help             Print help
```

### `stitch threads`

```text
List chat threads: this folder's (or project's), then global ones, most recent first

Usage: stitch threads [OPTIONS]

Options:
      --all          Every thread, other folders' and projects' too
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch thread`

```text
Print a thread's transcript (`last`: the latest here), switch it between read-only and writable, or show and change its tools and skills

Usage: stitch thread [OPTIONS] [ID]
       stitch thread <COMMAND>

Commands:
  tools   A thread's tools: listed, or changed first (`+github -web +github__get_*`; groups: shell, web, viewer, weave, memory)
  skills  A thread's skills: listed, or changed first (`-release +mail`)
  rm      Delete threads for good, on every node (design §2.5): each with its compaction chain, never the threads they spawned or their forks, which stay. Lists them and asks first
  help    Print this message or the help of the given subcommand(s)

Arguments:
  [ID]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --read-only    Make the thread read-only (from its next tool call): its shells write only temporary directories, only read-only commands run. Prints the mode
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --writable     Make the thread writable (from its next turn). Prints the mode
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch thread tools`

```text
A thread's tools: listed, or changed first (`+github -web +github__get_*`; groups: shell, web, viewer, weave, memory)

Usage: stitch thread tools [OPTIONS] <ID> [CHANGES]...

Arguments:
  <ID>          A thread ID, or `last`
  [CHANGES]...

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch thread skills`

```text
A thread's skills: listed, or changed first (`-release +mail`)

Usage: stitch thread skills [OPTIONS] <ID> [CHANGES]...

Arguments:
  <ID>
  [CHANGES]...

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch thread rm`

```text
Delete threads for good, on every node (design §2.5): each with its compaction chain, never the threads they spawned or their forks, which stay. Lists them and asks first

Usage: stitch thread rm [OPTIONS] [IDS]...

Arguments:
  [IDS]...  Thread IDs (`last`: the latest here)

Options:
      --data <DATA>              The node's data directory [env: STITCH_DATA]
      --spawned-by <SPAWNED_BY>  Also every thread this one spawned (not the threads they spawned)
      --root <ROOT>              The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
  -y, --yes                      Do not ask
      --node <NODE>              Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help                     Print help
```

### `stitch key`

```text
Provider API keys: secrets by the provider's name (`stitch secret`)

Usage: stitch key [OPTIONS] <COMMAND>

Commands:
  set   Store a provider's key (read without echo, or from standard input): `stitch secret set <provider>`
  rm    Remove a provider's key: `stitch secret rm <provider>`
  help  Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch key set`

```text
Store a provider's key (read without echo, or from standard input): `stitch secret set <provider>`

Usage: stitch key set [OPTIONS] <PROVIDER>

Arguments:
  <PROVIDER>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --file         In `credentials.toml` instead of the keychain
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch key rm`

```text
Remove a provider's key: `stitch secret rm <provider>`

Usage: stitch key rm [OPTIONS] <PROVIDER>

Arguments:
  <PROVIDER>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch secret`

```text
Secrets (design §2.19): kept in the keychain, synced sealed to your other nodes unless this device's only, given to shells by `[env]`, never shown to a model

Usage: stitch secret [OPTIONS] <COMMAND>

Commands:
  set   Keep a secret (read without echo, or from standard input); synced sealed to your other nodes unless `--device-only`
  get   Whether this node holds a secret; its value with `--reveal`
  ls    The secrets: where each is held and used (never their values)
  rm    Remove a secret here and, unless this device's only, everywhere
  help  Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch secret set`

```text
Keep a secret (read without echo, or from standard input); synced sealed to your other nodes unless `--device-only`

Usage: stitch secret set [OPTIONS] <NAME>

Arguments:
  <NAME>

Options:
      --data <DATA>    The node's data directory [env: STITCH_DATA]
      --scope <SCOPE>  `global` (the default), `project:<repo>` or `folder:<key>` [default: global]
      --here           The place the current directory is in (a project or a folder)
      --root <ROOT>    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --device-only    Kept on this device only, never synced (removed from the others)
      --node <NODE>    Overrides the node name from config.toml [env: STITCH_NODE]
      --sync           Synced to your other nodes (a secret that was this device's only)
      --keep-value     Change only `--device-only` or `--sync`, keeping the value
      --file           In `credentials.toml` instead of the keychain
  -h, --help           Print help
```

#### `stitch secret get`

```text
Whether this node holds a secret; its value with `--reveal`

Usage: stitch secret get [OPTIONS] <NAME>

Arguments:
  <NAME>

Options:
      --data <DATA>    The node's data directory [env: STITCH_DATA]
      --scope <SCOPE>  `global` (the default), `project:<repo>` or `folder:<key>` [default: global]
      --here           The place the current directory is in (a project or a folder)
      --root <ROOT>    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>    Overrides the node name from config.toml [env: STITCH_NODE]
      --reveal
  -h, --help           Print help
```

#### `stitch secret ls`

```text
The secrets: where each is held and used (never their values)

Usage: stitch secret ls [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch secret rm`

```text
Remove a secret here and, unless this device's only, everywhere

Usage: stitch secret rm [OPTIONS] <NAME>

Arguments:
  <NAME>

Options:
      --data <DATA>    The node's data directory [env: STITCH_DATA]
      --scope <SCOPE>  `global` (the default), `project:<repo>` or `folder:<key>` [default: global]
      --here           The place the current directory is in (a project or a folder)
      --root <ROOT>    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>    Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help           Print help
```

### `stitch vault`

```text
The node's opt-in end-to-end encrypted vault: backup and store-and-forward (docs/vault-protocol.md; experimental, no hosted service yet)

Usage: stitch vault [OPTIONS] <COMMAND>

Commands:
  create   Create a vault at a vault service and make this node its first admin. Prints the recovery kit once: keep it offline (it is the vault's root of trust)
  join     Join an existing vault. With --kit (read from standard input or a prompt), the kit admits this node at once; otherwise this prints the keys an admin passes to `stitch vault admit`
  admit    Admit another device (this node must be an admin). An admin admitted this way is a writer for 7 days first
  status   This node's standing in the vault, pending destructive actions, the last backup
  backup   Back up every record root and the original blobs they attach (incremental)
  recover  Recover into a new staging directory and write an inert preview (nothing in it runs; import chosen conversations with `stitch vault import`)
  import   Import records (and a thread's transcript) from a preview into a record root
  disable  Stop using the vault on this node (its directory, device key included, is moved aside; nothing changes in the vault)
  help     Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch vault create`

```text
Create a vault at a vault service and make this node its first admin. Prints the recovery kit once: keep it offline (it is the vault's root of trust)

Usage: stitch vault create [OPTIONS] --endpoint <ENDPOINT>

Options:
      --data <DATA>          The node's data directory [env: STITCH_DATA]
      --endpoint <ENDPOINT>
      --root <ROOT>          The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>          Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help                 Print help
```

#### `stitch vault join`

```text
Join an existing vault. With --kit (read from standard input or a prompt), the kit admits this node at once; otherwise this prints the keys an admin passes to `stitch vault admit`

Usage: stitch vault join [OPTIONS] --endpoint <ENDPOINT> --vault <VAULT>

Options:
      --data <DATA>          The node's data directory [env: STITCH_DATA]
      --endpoint <ENDPOINT>
      --root <ROOT>          The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --vault <VAULT>
      --kit
      --node <NODE>          Overrides the node name from config.toml [env: STITCH_NODE]
      --role <ROLE>          admin, writer or reader (with --kit; default writer) [default: writer]
  -h, --help                 Print help
```

#### `stitch vault admit`

```text
Admit another device (this node must be an admin). An admin admitted this way is a writer for 7 days first

Usage: stitch vault admit [OPTIONS] <DEVICE> <SEAL>

Arguments:
  <DEVICE>
  <SEAL>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --role <ROLE>  [default: writer]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch vault status`

```text
This node's standing in the vault, pending destructive actions, the last backup

Usage: stitch vault status [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --json
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch vault backup`

```text
Back up every record root and the original blobs they attach (incremental)

Usage: stitch vault backup [OPTIONS]

Options:
      --data <DATA>    The node's data directory [env: STITCH_DATA]
      --watch <WATCH>  Keep running: back up the repositories the change feed reports, checking every this many seconds
      --root <ROOT>    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>    Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help           Print help
```

#### `stitch vault recover`

```text
Recover into a new staging directory and write an inert preview (nothing in it runs; import chosen conversations with `stitch vault import`)

Usage: stitch vault recover [OPTIONS] --preview <PREVIEW>

Options:
      --data <DATA>          The node's data directory [env: STITCH_DATA]
      --preview <PREVIEW>    Where the inert preview goes (must not exist)
      --root <ROOT>          The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --staging <STAGING>    The staging directory (default: <preview>.staging); with --resume, the one a stopped recovery left
      --node <NODE>          Overrides the node name from config.toml [env: STITCH_NODE]
      --through <THROUGH>    The state as of this cursor (default: the head)
      --resume
      --phone                Phone-sized limits (per-run download bound; resume to continue)
      --kit                  Recover with the recovery kit (no device of the vault on this node)
      --endpoint <ENDPOINT>  With --kit: the vault service
  -h, --help                 Print help
```

#### `stitch vault import`

```text
Import records (and a thread's transcript) from a preview into a record root

Usage: stitch vault import [OPTIONS] --id <IDS> <PREVIEW>

Arguments:
  <PREVIEW>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --id <IDS>     Record IDs as the preview names them (repeatable)
      --root <ROOT>  The record root to import into (default: the home workspace)
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch vault disable`

```text
Stop using the vault on this node (its directory, device key included, is moved aside; nothing changes in the vault)

Usage: stitch vault disable [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch proxy`

```text
A key-holding proxy for model APIs: clients (a bot's node in a container) get tokens of their own, with budgets, and never the key

Usage: stitch proxy [OPTIONS] [COMMAND]

Commands:
  serve  Serve (the default): forward model requests with this machine's keys
  token  Tokens for clients
  usage  Each token's limits, and what it and its end users used by day
  help   Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --dir <DIR>    Where its configuration, usage and log are (default: `proxy` in the data directory) [env: STITCH_PROXY_DIR]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch proxy serve`

```text
Serve (the default): forward model requests with this machine's keys

Usage: stitch proxy serve [OPTIONS]

Options:
      --data <DATA>      The node's data directory [env: STITCH_DATA]
      --dir <DIR>        Where its configuration, usage and log are (default: `proxy` in the data directory) [env: STITCH_PROXY_DIR]
      --listen <LISTEN>  Addresses to listen on (default: `listen` in proxy.toml, else 127.0.0.1:7178 and Docker's bridge gateway 172.17.0.1:7178)
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help             Print help
```

#### `stitch proxy token`

```text
Tokens for clients

Usage: stitch proxy token [OPTIONS] <COMMAND>

Commands:
  add   Make a token (printed once; only its hash is kept), replacing one of the name
  ls    The tokens and their limits
  rm
  help  Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --dir <DIR>    Where its configuration, usage and log are (default: `proxy` in the data directory) [env: STITCH_PROXY_DIR]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

##### `stitch proxy token add`

```text
Make a token (printed once; only its hash is kept), replacing one of the name

Usage: stitch proxy token add [OPTIONS] <NAME>

Arguments:
  <NAME>

Options:
      --daily-tokens <DAILY_TOKENS>
          Budget tokens a day (UTC), all its users together
      --data <DATA>
          The node's data directory [env: STITCH_DATA]
      --dir <DIR>
          Where its configuration, usage and log are (default: `proxy` in the data directory) [env: STITCH_PROXY_DIR]
      --daily-tokens-per-user <DAILY_TOKENS_PER_USER>
          Budget tokens a day for each end user it names
      --root <ROOT>
          The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>
          Overrides the node name from config.toml [env: STITCH_NODE]
      --rpm <RPM>
          Requests a minute (default 30)
      --upstream <UPSTREAMS>
          Upstreams it may use (default: all)
  -h, --help
          Print help
```

##### `stitch proxy token ls`

```text
The tokens and their limits

Usage: stitch proxy token ls [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --dir <DIR>    Where its configuration, usage and log are (default: `proxy` in the data directory) [env: STITCH_PROXY_DIR]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

##### `stitch proxy token rm`

```text
Usage: stitch proxy token rm [OPTIONS] <NAME>

Arguments:
  <NAME>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --dir <DIR>    Where its configuration, usage and log are (default: `proxy` in the data directory) [env: STITCH_PROXY_DIR]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch proxy usage`

```text
Each token's limits, and what it and its end users used by day

Usage: stitch proxy usage [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --days <DAYS>  [default: 7]
      --dir <DIR>    Where its configuration, usage and log are (default: `proxy` in the data directory) [env: STITCH_PROXY_DIR]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch discord`

```text
A Discord bot in front of this node (`DISCORD_TOKEN` or `DISCORD_TOKEN_FILE`): a mention or `/ask` in an allowed guild starts a thread, answered in a Discord thread

Usage: stitch discord [OPTIONS]

Options:
      --data <DATA>
          The node's data directory [env: STITCH_DATA]
      --guilds <GUILDS>
          The guild IDs it answers in (commas or spaces); none: nowhere [env: STITCH_DISCORD_GUILDS] [default: ""]
      --root <ROOT>
          The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --turns-per-hour <TURNS_PER_HOUR>
          Turns each Discord user may start an hour [env: STITCH_DISCORD_TURNS_PER_HOUR] [default: 20]
      --model <MODEL>
          `provider/model`; the node's default when unset [env: STITCH_DISCORD_MODEL]
      --node <NODE>
          Overrides the node name from config.toml [env: STITCH_NODE]
      --announce
          Say so in each allowed guild's system channel when connected [env: STITCH_DISCORD_ANNOUNCE]
      --servers <SERVERS>
          A directory of per-server instructions, `<guild id>.md`, given in the first message of each new thread in that server [env: STITCH_DISCORD_SERVERS]
      --approvers <APPROVERS>
          Discord user IDs (commas or spaces) whose messages the agent sees tagged `[approver]`, a tag no one else can produce [env: STITCH_DISCORD_APPROVERS] [default: ""]
  -h, --help
          Print help
```

### `stitch var`

```text
Variables: values that are not secret, synced as records, given to shells by their names

Usage: stitch var [OPTIONS] <COMMAND>

Commands:
  set   Where a secret or variable applies
  ls
  rm    Where a secret or variable applies
  help  Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch var set`

```text
Where a secret or variable applies

Usage: stitch var set [OPTIONS] <NAME> <VALUE>

Arguments:
  <NAME>
  <VALUE>

Options:
      --data <DATA>    The node's data directory [env: STITCH_DATA]
      --scope <SCOPE>  `global` (the default), `project:<repo>` or `folder:<key>` [default: global]
      --here           The place the current directory is in (a project or a folder)
      --root <ROOT>    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>    Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help           Print help
```

#### `stitch var ls`

```text
Usage: stitch var ls [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch var rm`

```text
Where a secret or variable applies

Usage: stitch var rm [OPTIONS] <NAME>

Arguments:
  <NAME>

Options:
      --data <DATA>    The node's data directory [env: STITCH_DATA]
      --scope <SCOPE>  `global` (the default), `project:<repo>` or `folder:<key>` [default: global]
      --here           The place the current directory is in (a project or a folder)
      --root <ROOT>    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>    Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help           Print help
```

### `stitch model`

```text
Models, ranked as agents see them, and your settings for them (reserved, tier, price, note), synced to every node

Usage: stitch model [OPTIONS] <COMMAND>

Commands:
  ls       The models with prices, and what you set
  default  The default model, for every node (alone: says it); a node's `config.toml` `default_model` wins on it, a project's `.stitch/settings.toml` `[models] default` in it
  quick    The quick model agents spawn quick work on (`model: "quick"`), for every node (alone: says it); a node's `quick_model` wins on it, `[models] quick` of a project in it
  set      Set what is given for models matching a pattern (`*` matches anything): `stitch model set 'anthropic/claude-fable-*' --reserved`
  rm       Remove the settings of a pattern
  help     Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch model ls`

```text
The models with prices, and what you set

Usage: stitch model ls [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --node <NODE>  Another node's models
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
  -h, --help         Print help
```

#### `stitch model default`

```text
The default model, for every node (alone: says it); a node's `config.toml` `default_model` wins on it, a project's `.stitch/settings.toml` `[models] default` in it

Usage: stitch model default [OPTIONS] [MODEL]

Arguments:
  [MODEL]  `provider/model`

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --unset
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch model quick`

```text
The quick model agents spawn quick work on (`model: "quick"`), for every node (alone: says it); a node's `quick_model` wins on it, `[models] quick` of a project in it

Usage: stitch model quick [OPTIONS] [MODEL]

Arguments:
  [MODEL]  `provider/model`

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --unset
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch model set`

```text
Set what is given for models matching a pattern (`*` matches anything): `stitch model set 'anthropic/claude-fable-*' --reserved`

Usage: stitch model set [OPTIONS] <PATTERN>

Arguments:
  <PATTERN>

Options:
      --data <DATA>            The node's data directory [env: STITCH_DATA]
      --reserved [<RESERVED>]  Agents spawn on it only when you asked for it by name (`--reserved false` to undo) [possible values: true, false]
      --price <PRICE>          USD per million tokens, input/output (`2/12`); empty clears it
      --root <ROOT>            The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>            Overrides the node name from config.toml [env: STITCH_NODE]
      --note <NOTE>            Shown beside it; empty clears it
  -h, --help                   Print help
```

#### `stitch model rm`

```text
Remove the settings of a pattern

Usage: stitch model rm [OPTIONS] <PATTERN>

Arguments:
  <PATTERN>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch ssh`

```text
SSH hosts (design §2.20): records every node knows, the shell tools' `host`; their host keys; this device's key and authorizing it

Usage: stitch ssh [OPTIONS] <COMMAND>

Commands:
  host            SSH hosts: add (or replace), list, remove
  key             This device's public key (made the first time), for a host's `authorized_keys`
  authorize       Add a device's public key to a host's `authorized_keys`: this node's (or `--node`'s, or `--key`), done here when this node gets in, else asked of the nodes that do
  authorizations  Requests to authorize keys, and how they went
  hostkey         Host keys trusted (on first use): list, forget
  help            Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch ssh host`

```text
SSH hosts: add (or replace), list, remove

Usage: stitch ssh host [OPTIONS] <COMMAND>

Commands:
  add   Add a host, or replace the one of that name (a record every node knows)
  ls
  rm
  help  Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

##### `stitch ssh host add`

```text
Add a host, or replace the one of that name (a record every node knows)

Usage: stitch ssh host add [OPTIONS] <NAME>

Arguments:
  <NAME>

Options:
      --address <ADDRESS>    Where it is (default: the name)
      --data <DATA>          The node's data directory [env: STITCH_DATA]
      --port <PORT>
      --root <ROOT>          The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>          Overrides the node name from config.toml [env: STITCH_NODE]
      --user <USER>
      --jump <JUMP>          A host to go through (its name)
      --auth <AUTH>          auto (the agent and key files, then this device's key), agent, device, secret:<name> (a password), key:<path> [default: auto]
      --approval <APPROVAL>  relaxed (what reaches beyond asks), ask (what is not read-only asks), allow (trusted whole) [default: relaxed]
      --no-tmux              Plain shells, not tmux, even where the host has it
  -h, --help                 Print help
```

##### `stitch ssh host ls`

```text
Usage: stitch ssh host ls [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

##### `stitch ssh host rm`

```text
Usage: stitch ssh host rm [OPTIONS] <NAME>

Arguments:
  <NAME>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch ssh key`

```text
This device's public key (made the first time), for a host's `authorized_keys`

Usage: stitch ssh key [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch ssh authorize`

```text
Add a device's public key to a host's `authorized_keys`: this node's (or `--node`'s, or `--key`), done here when this node gets in, else asked of the nodes that do

Usage: stitch ssh authorize [OPTIONS] <HOST>

Arguments:
  <HOST>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --node <NODE>  The node whose published key to add
      --key <KEY>    An `authorized_keys` line to add
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
  -h, --help         Print help
```

#### `stitch ssh authorizations`

```text
Requests to authorize keys, and how they went

Usage: stitch ssh authorizations [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch ssh hostkey`

```text
Host keys trusted (on first use): list, forget

Usage: stitch ssh hostkey [OPTIONS] <COMMAND>

Commands:
  ls
  rm    Forget the keys of a host (its name, or `[address]:port`): the next connection trusts what it finds
  help  Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

##### `stitch ssh hostkey ls`

```text
Usage: stitch ssh hostkey ls [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

##### `stitch ssh hostkey rm`

```text
Forget the keys of a host (its name, or `[address]:port`): the next connection trusts what it finds

Usage: stitch ssh hostkey rm [OPTIONS] <HOST>

Arguments:
  <HOST>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch auth`

```text
Subscription sign-in (Anthropic, OpenAI/Codex, Grok)

Usage: stitch auth [OPTIONS] <COMMAND>

Commands:
  login   Sign in: a subscription (`anthropic`, `openai`, `grok`: a browser sign-in URL, then the local callback), or `github` (a code to enter at github.com/login/device)
  logout  Remove the local subscription session (API keys are separate)
  status  Show local sign-in state without printing or refreshing tokens
  help    Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch auth login`

```text
Sign in: a subscription (`anthropic`, `openai`, `grok`: a browser sign-in URL, then the local callback), or `github` (a code to enter at github.com/login/device)

Usage: stitch auth login [OPTIONS] <PROVIDER>

Arguments:
  <PROVIDER>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --file         Store tokens in an owner-only file instead of the keychain
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch auth logout`

```text
Remove the local subscription session (API keys are separate)

Usage: stitch auth logout [OPTIONS] <PROVIDER>

Arguments:
  <PROVIDER>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch auth status`

```text
Show local sign-in state without printing or refreshing tokens

Usage: stitch auth status [OPTIONS] [PROVIDER]

Arguments:
  [PROVIDER]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch apns`

```text
Push notifications to your phones, sent by this node through Apple's push service with your own key

Usage: stitch apns [OPTIONS] <COMMAND>

Commands:
  set     Keep an APNs key (the `.p8` from developer.apple.com, Keys) for this node to push with: in the keychain, its IDs in config.toml
  relay   Push through another relay than the app's (the Worker in relay/, the default for a node without a key of its own)
  test    Push a test notification to your phones
  status  Whether this node pushes, and the phones it would push to
  help    Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch apns set`

```text
Keep an APNs key (the `.p8` from developer.apple.com, Keys) for this node to push with: in the keychain, its IDs in config.toml

Usage: stitch apns set [OPTIONS] --key-id <KEY_ID> --team-id <TEAM_ID> <P8>

Arguments:
  <P8>  The `AuthKey_<id>.p8` file

Options:
      --data <DATA>        The node's data directory [env: STITCH_DATA]
      --key-id <KEY_ID>    The key's ID, as the developer site shows it
      --root <ROOT>        The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --team-id <TEAM_ID>  The team that made the key: the one that signs the app (project.yml's DEVELOPMENT_TEAM, ten characters)
      --file               In an owner-only file instead of the keychain
      --node <NODE>        Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help               Print help
```

#### `stitch apns relay`

```text
Push through another relay than the app's (the Worker in relay/, the default for a node without a key of its own)

Usage: stitch apns relay [OPTIONS] <URL>

Arguments:
  <URL>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch apns test`

```text
Push a test notification to your phones

Usage: stitch apns test [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch apns status`

```text
Whether this node pushes, and the phones it would push to

Usage: stitch apns status [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch app`

```text
Apps agents made (design §2.10): pinned ones, opened in the browser

Usage: stitch app [OPTIONS] <COMMAND>

Commands:
  list             The pinned apps, with their addresses here
  open             Open an app in the browser: a pinned one (by name or ID), the latest one a thread showed (by the thread's ID), or a version (`blake3:…`); with none, the list of pinned apps
  pin              Pin an app: the latest one a thread showed, or a version (`blake3:…`); pinned again from the same thread, it moves to the newer version. Pinned from a thread, it follows it: each newer version of its name the thread shows moves it. A version stays
  unpin            Unpin an app (by name or ID); its versions stay in their threads
  export           Write an app's files to a directory: a pinned app (by name or ID), the latest one a thread showed (by the thread's ID), or a version (`blake3:…`)
  allow-live       Let an app see your threads, jobs and approvals (`stitch.live`): a pinned app by name or ID, or an app in its conversation by its name and `--thread`. Opened in a browser, an app has no other way to ask
  allow-threads    Show what an app's `stitch-app.json` asks for threads of its own (places as resolved here, tools, skills, models, budgets) and the hash of those needs; grant exactly them with `--needs <hash>`. A changed manifest asks again. A pinned app by name or ID, or an app in its conversation by its exact name and `--thread`
  allow-container  Show what an app asks of containers (design §2.23): its build (the Containerfile, shown, or its change since the last build grant; the build runs the app's code with internet access), its commands, its shell, each with a hash; grant each with its hash. A changed Containerfile or tool asks again. Linux with rootless podman only
  storage          Apps' container images and caches on this node, with their sizes; or one app's removed (`--rm <name>`: its images and cache, rebuilt at its next run; its data folder stays)
  help             Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch app list`

```text
The pinned apps, with their addresses here

Usage: stitch app list [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch app open`

```text
Open an app in the browser: a pinned one (by name or ID), the latest one a thread showed (by the thread's ID), or a version (`blake3:…`); with none, the list of pinned apps

Usage: stitch app open [OPTIONS] [APP]

Arguments:
  [APP]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch app pin`

```text
Pin an app: the latest one a thread showed, or a version (`blake3:…`); pinned again from the same thread, it moves to the newer version. Pinned from a thread, it follows it: each newer version of its name the thread shows moves it. A version stays

Usage: stitch app pin [OPTIONS] <WHAT>

Arguments:
  <WHAT>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --name <NAME>  Its name (default: the app's own)
      --icon <ICON>  Its icon on the phone: an SF Symbol's name (`checklist`, `chart.bar`)
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch app unpin`

```text
Unpin an app (by name or ID); its versions stay in their threads

Usage: stitch app unpin [OPTIONS] <APP>

Arguments:
  <APP>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch app export`

```text
Write an app's files to a directory: a pinned app (by name or ID), the latest one a thread showed (by the thread's ID), or a version (`blake3:…`)

Usage: stitch app export [OPTIONS] <WHAT> [DIR]

Arguments:
  <WHAT>
  [DIR]   Where (default: `./<its name>`, made lower-case with dashes)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --force        Write into a directory that is not empty, replacing its files
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch app allow-live`

```text
Let an app see your threads, jobs and approvals (`stitch.live`): a pinned app by name or ID, or an app in its conversation by its name and `--thread`. Opened in a browser, an app has no other way to ask

Usage: stitch app allow-live [OPTIONS] <APP>

Arguments:
  <APP>

Options:
      --data <DATA>      The node's data directory [env: STITCH_DATA]
      --thread <THREAD>  The thread whose conversation shows the app (not pinned)
      --deny             Take it back: it may no longer
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help             Print help
```

#### `stitch app allow-threads`

```text
Show what an app's `stitch-app.json` asks for threads of its own (places as resolved here, tools, skills, models, budgets) and the hash of those needs; grant exactly them with `--needs <hash>`. A changed manifest asks again. A pinned app by name or ID, or an app in its conversation by its exact name and `--thread`

Usage: stitch app allow-threads [OPTIONS] <APP>

Arguments:
  <APP>

Options:
      --data <DATA>      The node's data directory [env: STITCH_DATA]
      --thread <THREAD>  The thread whose conversation shows the app (not pinned)
      --needs <NEEDS>    Grant the needs shown with this hash (refused when the app's latest version asks for others by now)
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --deny             Take it back: nothing more of its threads runs, those at work are cancelled
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help             Print help
```

#### `stitch app allow-container`

```text
Show what an app asks of containers (design §2.23): its build (the Containerfile, shown, or its change since the last build grant; the build runs the app's code with internet access), its commands, its shell, each with a hash; grant each with its hash. A changed Containerfile or tool asks again. Linux with rootless podman only

Usage: stitch app allow-container [OPTIONS] <APP>

Arguments:
  <APP>

Options:
      --data <DATA>      The node's data directory [env: STITCH_DATA]
      --thread <THREAD>  The thread whose conversation shows the app (not pinned)
      --build <BUILD>    Grant the build shown with this hash (the build starts)
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
      --run <RUN>        Grant the commands shown with this hash
      --shell <SHELL>    Grant the shell shown with this hash (it cancels pinning)
      --deny             Take them all back; its containers running are stopped
  -h, --help             Print help
```

#### `stitch app storage`

```text
Apps' container images and caches on this node, with their sizes; or one app's removed (`--rm <name>`: its images and cache, rebuilt at its next run; its data folder stays)

Usage: stitch app storage [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --rm <REMOVE>
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch dashboard`

```text
The dashboard: your threads on every node, what needs you, jobs, live; pinned (with the phone too) and opened in the browser

Usage: stitch dashboard [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch client`

```text
Open the web client (the node's own UI for a browser: conversations, home, approvals) and print its address

Usage: stitch client [OPTIONS]

Options:
      --data <DATA>      The node's data directory [env: STITCH_DATA]
      --no-open          Print the address only; open no browser
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --thread <THREAD>  Open on this conversation (`#/t/<id>`)
      --map              Open on the map of conversations (`#/map`)
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help             Print help
```

### `stitch loose-ends`

```text
Loose ends: the conversations that wait on you. Judges first (the decision model, the quick model where it is unsure, on those not judged at their latest step), then lists; the node does it hourly by itself (`[loose_ends]`)

Usage: stitch loose-ends [OPTIONS]

Options:
      --data <DATA>                   The node's data directory [env: STITCH_DATA]
      --list                          Only list, judging nothing
      --dismiss <DISMISS>             Hide this thread's loose end until it moves again
      --root <ROOT>                   The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --auto <AUTO>                   Auto-handling: `off`, `dry-run` (records what it would do) or `on` (continues or dismisses as your rules say); synced to every node [possible values: off, dry-run, on]
      --node <NODE>                   Overrides the node name from config.toml [env: STITCH_NODE]
      --rules <show|edit|clear|FILE>  Your rules for auto-handling: `show`, `edit` (in $EDITOR), `clear`, or a file to set them from (`-`: standard input)
  -h, --help                          Print help
```

### `stitch invite`

```text
Show a pairing ticket (QR code and text) for a new node

Usage: stitch invite [OPTIONS]

Options:
      --data <DATA>        The node's data directory [env: STITCH_DATA]
      --minutes <MINUTES>  Minutes the ticket stays valid [default: 10]
      --root <ROOT>        The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>        Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help               Print help
```

### `stitch join`

```text
Join the user's nodes with a ticket from `stitch invite`

Usage: stitch join [OPTIONS] <TICKET>

Arguments:
  <TICKET>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --home <HOME>  Where to hold the home workspace (default: `home` in the data directory)
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch sync`

```text
Sync with every known node now, or with one

Usage: stitch sync [OPTIONS] [PEER]

Arguments:
  [PEER]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch id`

```text
Print this node's endpoint ID

Usage: stitch id [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch peer`

```text
The nodes trusted by this one

Usage: stitch peer [OPTIONS] <COMMAND>

Commands:
  add     Trust a node by name and endpoint ID (pairing does this for you)
  ls
  remove  Stop trusting a node: a `revoke` record in the home workspace, which spreads to every node, and removal from the local list
  help    Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch peer add`

```text
Trust a node by name and endpoint ID (pairing does this for you)

Usage: stitch peer add [OPTIONS] <NAME> <ID>

Arguments:
  <NAME>
  <ID>

Options:
      --addr <ADDRS>  A direct address (`ip:port`), repeatable; needed without discovery
      --data <DATA>   The node's data directory [env: STITCH_DATA]
      --root <ROOT>   The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>   Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help          Print help
```

#### `stitch peer ls`

```text
Usage: stitch peer ls [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch peer remove`

```text
Stop trusting a node: a `revoke` record in the home workspace, which spreads to every node, and removal from the local list

Usage: stitch peer remove [OPTIONS] <NAME>

Arguments:
  <NAME>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch project`

```text
Projects: every git checkout is one, known by its first commit

Usage: stitch project [OPTIONS] <COMMAND>

Commands:
  ls      The projects the home workspace lists, and where they are checked out here
  split   Make the fork checked out here a project of its own (by its `origin`), rather than one with the project it was forked from
  rebind  Have the project of this checkout's origin follow it here, its repository re-created from a new first commit (same origin, a new history)
  help    Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch project ls`

```text
The projects the home workspace lists, and where they are checked out here

Usage: stitch project ls [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch project split`

```text
Make the fork checked out here a project of its own (by its `origin`), rather than one with the project it was forked from

Usage: stitch project split [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch project rebind`

```text
Have the project of this checkout's origin follow it here, its repository re-created from a new first commit (same origin, a new history)

Usage: stitch project rebind [OPTIONS]

Options:
      --data <DATA>        The node's data directory [env: STITCH_DATA]
      --project <PROJECT>  The project, by name or ID, when several have this origin
      --root <ROOT>        The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>        Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help               Print help
```

### `stitch browser`

```text
The user's own Chrome through Stitch's extension, and the headless Chromium the web tools drive

Usage: stitch browser [OPTIONS] <COMMAND>

Commands:
  install   Set up Stitch's extension for your own Chrome: write its files (to load unpacked) and the native messaging host for every Chromium browser found (Chrome, Chromium, Brave, Edge). `install headless` downloads the headless Chromium for the web tools instead
  profiles  The Chrome profiles that have connected, and whether Stitch is enabled in each; `disable <name>` turns one off (enabling takes a click in that profile's extension popup)
  call      Run a browser tool as a thread would, outside any turn (for trying things and tests): `stitch browser call browser_tabs '{"action":"new","url":"…"}'`
  help      Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch browser install`

```text
Set up Stitch's extension for your own Chrome: write its files (to load unpacked) and the native messaging host for every Chromium browser found (Chrome, Chromium, Brave, Edge). `install headless` downloads the headless Chromium for the web tools instead

Usage: stitch browser install [OPTIONS] [WHAT]

Arguments:
  [WHAT]  `headless`: the web tools' Chromium (about 100 MB)

Options:
      --data <DATA>                The node's data directory [env: STITCH_DATA]
      --profile-dir <PROFILE_DIR>  Write the host manifest into this browser user-data directory only (a test profile), not into the browsers found
      --root <ROOT>                The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>                Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help                       Print help
```

#### `stitch browser profiles`

```text
The Chrome profiles that have connected, and whether Stitch is enabled in each; `disable <name>` turns one off (enabling takes a click in that profile's extension popup)

Usage: stitch browser profiles [OPTIONS] [ACTION] [NAME]

Arguments:
  [ACTION]  `disable`
  [NAME]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch browser call`

```text
Run a browser tool as a thread would, outside any turn (for trying things and tests): `stitch browser call browser_tabs '{"action":"new","url":"…"}'`

Usage: stitch browser call [OPTIONS] <TOOL> [INPUT]

Arguments:
  <TOOL>
  [INPUT]  [default: {}]

Options:
      --data <DATA>      The node's data directory [env: STITCH_DATA]
      --thread <THREAD>  The session's owner (default: `cli`, one session shared by calls)
      --images <IMAGES>  Where images go (default: printed paths in the downloads folder)
      --root <ROOT>      The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>      Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help             Print help
```

### `stitch memory`

```text
Memory: what was said and seen, recalled across conversations

Usage: stitch memory [OPTIONS] <COMMAND>

Commands:
  install  Download EmbeddingGemma 2 into the data directory (about 310 MB, and 555 MB more for images), which turns memory on
  status   Whether memory runs here, and how much it holds
  audit    The drift check: decisions by novelty band, and strengths (run it after any change of thresholds or model)
  notes    Every unasked note's memories, tab-separated: the cue's measures, each memory's raw and centered distance, and whether it helped, was dismissed or ignored (the data to recalibrate the note gates with)
  off      Take a thread out of memory: it is no longer remembered, and what it held is no longer recalled anywhere (reversible)
  on       Bring a thread back into memory, remembering what it holds
  help     Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch memory install`

```text
Download EmbeddingGemma 2 into the data directory (about 310 MB, and 555 MB more for images), which turns memory on

Usage: stitch memory install [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --text-only    Remember text only: skip the vision projector
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch memory status`

```text
Whether memory runs here, and how much it holds

Usage: stitch memory status [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch memory audit`

```text
The drift check: decisions by novelty band, and strengths (run it after any change of thresholds or model)

Usage: stitch memory audit [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch memory notes`

```text
Every unasked note's memories, tab-separated: the cue's measures, each memory's raw and centered distance, and whether it helped, was dismissed or ignored (the data to recalibrate the note gates with)

Usage: stitch memory notes [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch memory off`

```text
Take a thread out of memory: it is no longer remembered, and what it held is no longer recalled anywhere (reversible)

Usage: stitch memory off [OPTIONS] [THREAD]

Arguments:
  [THREAD]  The thread; the latest of this place when omitted

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch memory on`

```text
Bring a thread back into memory, remembering what it holds

Usage: stitch memory on [OPTIONS] [THREAD]

Arguments:
  [THREAD]  The thread; the latest of this place when omitted

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch folder`

```text
Folders outside git: the same `~` path is the same folder on every node; these join or part others

Usage: stitch folder [OPTIONS] <COMMAND>

Commands:
  link    Make this folder the same as another: `node:path`, or a path here. The other must be a folder Stitch knows (a directory here, or one a thread, a folder record or a folder sync shows on its node)
  unlink  Keep this folder to this node, apart from same-named folders on others and from its links
  sync    Sync this folder's files with the user's other machines now (it happens before and after each turn here, and after each sync). A new folder syncs only once the user lets it: `--allow`, or the question its first thread asks; `--stop` keeps its files on each node
  log     This folder's changes, the latest first: when, on which node, by which thread, what they changed; only a file's (or directory's) when given
  diff    What a change did, as a diff: the latest by default (`--to`), against what it follows (`--from`); only a file's when given
  help    Print this message or the help of the given subcommand(s)

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch folder link`

```text
Make this folder the same as another: `node:path`, or a path here. The other must be a folder Stitch knows (a directory here, or one a thread, a folder record or a folder sync shows on its node)

Usage: stitch folder link [OPTIONS] <OTHER>

Arguments:
  <OTHER>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --force        Link even a location Stitch does not know
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch folder unlink`

```text
Keep this folder to this node, apart from same-named folders on others and from its links

Usage: stitch folder unlink [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch folder sync`

```text
Sync this folder's files with the user's other machines now (it happens before and after each turn here, and after each sync). A new folder syncs only once the user lets it: `--allow`, or the question its first thread asks; `--stop` keeps its files on each node

Usage: stitch folder sync [OPTIONS]

Options:
      --allow
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --stop
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

#### `stitch folder log`

```text
This folder's changes, the latest first: when, on which node, by which thread, what they changed; only a file's (or directory's) when given

Usage: stitch folder log [OPTIONS] [PATH]

Arguments:
  [PATH]

Options:
      --data <DATA>    The node's data directory [env: STITCH_DATA]
  -n, --limit <LIMIT>  [default: 20]
      --root <ROOT>    The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>    Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help           Print help
```

#### `stitch folder diff`

```text
What a change did, as a diff: the latest by default (`--to`), against what it follows (`--from`); only a file's when given

Usage: stitch folder diff [OPTIONS] [PATH]

Arguments:
  [PATH]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --from <FROM>
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --to <TO>
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch new`

```text
Write a new record and print its ID

Usage: stitch new [OPTIONS] <KIND>

Arguments:
  <KIND>

Options:
      --data <DATA>           The node's data directory [env: STITCH_DATA]
      --title <TITLE>         Becomes the body's first heading
      --body <BODY>           The body; `-` reads it from standard input
      --root <ROOT>           The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>           Overrides the node name from config.toml [env: STITCH_NODE]
      --ref <REFS>            `rel=[repo:]id[#sub]`, repeatable
      --field <FIELDS>        `key=value`, the value in TOML (a bare word is a string), repeatable
      --attach <ATTACHMENTS>  A file to put in the blob store and attach, repeatable
      --user                  Written by the user rather than by the node
  -h, --help                  Print help
```

### `stitch show`

```text
Print a record and its derived status

Usage: stitch show [OPTIONS] <ID>

Arguments:
  <ID>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch ls`

```text
List records, oldest first

Usage: stitch ls [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --kind <KIND>
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch attention`

```text
What is blocked on the user

Usage: stitch attention [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch edges`

```text
Every edge of the graph, as `src rel dst`

Usage: stitch edges [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch sync-local`

```text
Reconcile the root with another record root on this machine

Usage: stitch sync-local [OPTIONS] <OTHER>

Arguments:
  <OTHER>

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```

### `stitch reindex`

```text
Drop the index and rebuild it from the home workspace and the projects

Usage: stitch reindex [OPTIONS]

Options:
      --data <DATA>  The node's data directory [env: STITCH_DATA]
      --root <ROOT>  The record root for record commands (default: the home workspace) [env: STITCH_ROOT]
      --node <NODE>  Overrides the node name from config.toml [env: STITCH_NODE]
  -h, --help         Print help
```
