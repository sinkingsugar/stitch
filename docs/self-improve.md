# Self-improvement: Stitch fixing and upgrading itself (proposal)

Status: **decided 2026-10-28** (the proposal, with the user's answers in §4;
being built in the order of §3, each step's state noted there). Design §4
question 10 points here.

The goal: an agent that notices a bug in Stitch while working (a "harness
issue") or sees an improvement can get it fixed, tested, landed and running on
the user's nodes (tower, mini, laptop, iphone) with little friction, without
breaking the fleet and without killing the work that is in flight, the
thread doing the upgrade included.

## 1. How upgrades and restarts work today

> **Historical baseline, 2026-10-28.** This section records the state used to
> design the changes below, not the current install/update procedure. Build
> identity, retained builds, health checks, rollback and the repository-owned
> update service have since been implemented. For current commands, use
> [setup §1](setup.md#1-install-update-restart); implementation progress is
> recorded in §3 of this document. The original observations are retained to
> explain the decisions.

### 1.1 Building and installing

- `just install` = `cargo install --path crates/stitch-cli` (macOS: then
  codesign with the Apple Development identity so keychain grants survive).
  It **overwrites** `~/.cargo/bin/stitch` and `stitch-shell-helper` in place:
  no previous binary is kept, so there is nothing to roll back to.
- `just update` (justfile): `git pull --ff-only`, submodules, `just
  install` (`--locked`: Otter and engram as `Cargo.lock` pins them, since
  2026-11-21; before, pulls of `../engram` and `../otter`), `just start`.
  `just update-remote <host>` runs it over `ssh -t` in a login shell.
- Nothing records **which build** a node runs: `VERSION` is
  `CARGO_PKG_VERSION + "/local-N"` (`crates/stitch-node/src/lib.rs:72`), the
  local protocol number, bumped only when `api` changes shape (enforced by
  `API_SHAPE`'s test). Two builds of different commits with the same `api`
  report the same version.

### 1.2 Hand-over between versions

- Every CLI command is a client: `Client::connect_or_start`
  (`crates/stitch-node/src/client.rs:197`) greets the node with
  `Request::Hello { version }`; with no node it spawns
  `current_exe() --data <data> node` (own process group, output appended to
  `<data>/node.log`; on macOS from the browser host, a launchd job of the GUI
  session, `launchd_start`, `<data>/node.plist`).
- The node (`node.rs:4250`): a client of the **same** version is served; a
  **newer** one makes it stop at once when idle, else `restart_when_idle`
  (`node.rs:318`) polls `busy()` every 2 s and stops once nothing runs; an
  older client is refused. `busy()` (`node.rs:274`) = chats/loops at work
  (detached threads waiting on an approval or `wait` excepted), tool calls
  for other nodes, commands running in shells.
- Stopping (`Request::Stop`, SIGTERM; `node.rs:2057`, `:5022`, `:2380`):
  loops are aborted (`Detachments::abort_all`, `detached.rs:1504`), open
  turns sealed, connections, MCP servers, shells and SSH sessions closed.
- At start, `detached::resume_all` (`detached.rs:2670`) resumes this node's
  detached threads with no report (their cut-off turn resumes with
  `chat::resume`); a question left open stays open and its answer, even one
  given while the node was down, is used. **Chats do not resume** (their
  questions are withdrawn); the iOS app resumes the chats it suspended
  (`Steer::suspend`, `chat.rs:291`).

### 1.3 Per node, as found

| Node | How the node runs | How it is updated |
|---|---|---|
| tower | started by whichever client came first (`~/.cargo/bin/stitch --data ~/.stitch node`, up since 2026-10-27 11:18); no unit (only `stitch-proxy.service`) | by hand: `just update` / `cargo install` + `stitch node stop` |
| mini (devbox-1) | **systemd user unit** `stitch.service` (`Restart=on-failure`, explicit `PATH`) | **`stitch-update.timer`** every 30 min → `~/.local/bin/stitch-self-update`: pull stitch/engram/otter, build only when changed, wait up to 2 h for `stitch jobs` to say nothing runs, `cargo install`, stop the unit (and any node outside it), start it. **Not in this repository.** |
| laptop | node started by clients or by launchd (`node.plist`, from the browser host) | by hand, `just update` (+ signing) |
| iphone | the node is linked into the app (`stitch-ffi`) | from the Mac only: `just ios device` then `just phone` (Xcode, the phone connected/paired, `devicectl`); reinstalling the app ends its node, which resumes suspended chats when in front again |

CI (`.github/workflows/ci.yml`) then ran the Linux checks on devbox-1 on
every pushed branch; Apple builds and UI tests ran only in `just
merge-check` on a Mac. (The app's builds and UI tests are the app
repository's now, §6.)

### 1.4 Pain points

1. **A rebuild without a protocol bump does not take effect.**
   `just update` ends with `just start`, which connects to the old node
   (same `VERSION`) and keeps running old code until someone runs
   `stitch node stop`. Nobody can tell what build a node runs.
2. **Restarting cuts work.** `stitch node stop` aborts every loop mid-step
   and kills every shell: a 10-minute `cargo build` in a thread is lost and
   redone on resume, chats are dropped. The gentle path
   (`restart_when_idle`) only exists for a newer *protocol*, waits for the
   whole node to be idle (with schedules and many threads it may never be),
   and does not stop new turns from starting meanwhile (starvation).
3. **The thread doing the upgrade is the obstacle.** An agent that runs the
   upgrade from its own shell keeps the node busy (its own command is
   running), and `stitch node stop` from that shell kills the agent's own
   turn mid-tool-call.
4. **A node restarted from a thread's shell inherits the sandbox.**
   `connect_or_start` spawns the node as a child of the calling process; from
   a sandboxed session that is a Landlock (Linux) / Seatbelt (macOS)
   confined process, and every shell and write of the new node would be
   confined too. Its environment (PATH, keys) is the thread shell's too.
5. **No supervisor, no health check, no rollback** (but on mini). If the new
   binary fails to start (a migration error, a panic), nothing notices and
   nothing goes back; the old binary is gone (`cargo install` overwrote it).
   Migrations run on first start (`migrate.rs`) and are only backed up for
   the segment migration.
6. **Stale clients.** Long-lived TUIs keep running deleted binaries (on tower
   three TUIs from 2026-10-25 run `~/.cargo/bin/stitch (deleted)`); after a
   protocol bump they are refused, and if one is the first to need a node it
   tries to start `current_exe()`, which is the deleted path.
7. **Each node is updated differently** (by hand, a private timer script,
   launchd, Xcode), and only by someone at a shell on that machine. The
   phone needs the Mac, Xcode and the phone at hand.
8. **No place for "I found a bug in Stitch".** An agent that hits a harness
   bug works around it, and the knowledge is lost in a transcript.

## 2. Proposal

Three layers, each useful alone:

1. **Upgrades that are safe by construction** (Rust, in the node and CLI):
   build identity, staged binaries, drain at step boundaries, hand-over by
   the node itself, health check, automatic rollback. `stitch node upgrade`
   and, across nodes, `stitch fleet upgrade`.
2. **Harness issues** as a cheap, first-class thing any thread can file.
3. **The self-improve loop**: a built-in `stitch-dev` skill (procedure) plus
   a schedule that turns issues into fixer threads; the fixer works in a
   worktree, passes the gates, opens a PR, lands on approval, and calls
   layer 1 to roll out.

### 2.1 Layer 1: node upgrade

**Build identity.** `BUILD` = git commit (+`-dirty`), Otter's and
engram's commits as `Cargo.lock` pins them (git dependencies since
2026-11-21; "(linked)" with the local checkout's commit under `just
dev-link`; `tools/build-label.sh` says the same for the shell), and the build time, set by a `build.rs` in
`stitch-cli` (or `just` passing env). Shown in `stitch node status`,
`stitch --version`, the TUI's status line, and in what peers learn of each
other (so `stitch fleet status` shows every node's build). Asked with a new
`Request::Build` after `Hello` (Hello's shape stays fixed).

**Staged binaries.** Builds are installed side by side, never over the
running one:

```
<data>/bin/<build>/stitch, stitch-shell-helper   # one directory per build
<data>/bin/current  -> <build>                    # what starts
<data>/bin/previous -> <build>                    # the rollback target
~/.cargo/bin/stitch -> <data>/bin/current/stitch  # (or a 3-line shim)
```

`stitch node stage [--from <path>|--build <checkout>]` copies (macOS:
signs) a built binary in and checks it runs (`<new> --version`, the helper's
protocol handshake). Keep the last 3 builds.

**Supervisor.** `stitch node service install` writes a systemd user unit
(mini's, generalised: explicit PATH, `Restart=on-failure`) or a LaunchAgent
(KeepAlive, the GUI domain, as `launchd_start` already does), starting
`<data>/bin/current/stitch node`. Optional but recommended: with it, a crash
restarts the node and a restart never inherits a client's sandbox or
environment. Without it, the node spawns its successor itself (below), in
its own session (`setsid`), never as a child of a client.

**Upgrade, run by the node** (`Request::Upgrade { to: build, drain_secs }`,
`stitch node upgrade [<build>] [--drain 600] [--now]`):

1. *Preflight.* The staged binary runs `stitch --data <tmp> node
   --selftest`: opens a fresh store, index, the API socket, the Otter tool
   host, answers `Hello`, exits 0. (Seconds; no network.)
2. *Drain.* The node stops admitting new turns (detached starts, schedule
   firings and wake-ups are deferred, not dropped; new chats are told "the
   node is upgrading, a moment"). Every running loop gets
   `Steer::suspend_at_step()`: it finishes the model call or tool call in
   flight and stops at the next step boundary, recorded `why = "suspended"`,
   so it resumes exactly there. Shell commands running for a thread get
   `drain_secs` (default 10 min) to end; past it they are cut (as today).
   Threads waiting on approvals or `wait` are not busy (as today).
3. *Hand-over.* The node writes `<data>/upgrade.json` (`{from, to, at,
   by: <thread>, state: "probation", attempts}`), seals turns, closes as
   today, repoints `current`, and exits; the supervisor (or the successor it
   spawned with `setsid`) starts the new build.
4. *Health.* The new node, seeing `probation`, checks within 60 s: store and
   index open, migrations done, socket answers, iroh endpoint bound, at
   least one peer reached (when any was reachable before), detached threads
   resumed without error, the shell helper's handshake. Passing, it writes
   `state: "healthy"` and keeps `previous`. A **watcher** (the old binary,
   started by the old node as `stitch node upgrade-watch` in its own
   session) waits for `healthy`; on timeout, a crash loop (2 starts without
   health), or the node not answering, it stops the new node, points
   `current` back at `previous`, starts it, and records the failure.
5. *Report.* The outcome (`upgraded A → B in 41 s`, or `rolled back: <log
   tail>`) is a notice to the user and a message to the thread that asked
   (below).

**Migrations and rollback.** Rollback is only safe when the old build can
read what the new one wrote. Rule to adopt: a migration that changes files
on disk keeps a copy (as `migrate.rs` does), and a build records the store
format it needs; the watcher refuses to roll back past a format change and
says so instead (the user decides). Most upgrades change no format.

**Not killing the upgrading thread.** The tool returns at once
(`upgrade u-12 scheduled: the node drains, then restarts; you will be woken
with the result`), the thread's next step boundary suspends it like the
others, and after the restart it resumes (detached threads already do) and
is woken with the outcome, through the same path as `wake_at`. A *chat*
asking for an upgrade gets the same answer, and the TUI reconnects
(re-exec'ing itself when it is of an older build: fixes stale TUIs).

**Across nodes** (`stitch fleet upgrade --commit <sha> [--nodes ...]`, and
`stitch fleet status`): each desktop node builds its own binary (macOS cannot
be cross-built here, and each must sign its own). The request goes to the
peer over the existing node protocol as a new remote question
`Upgrade { commit }`, which the target answers only from its own nodes and
only within an `[upgrade]` policy (`accept_from = ["tower"]`, branches
allowed: `main` only by default). The target fetches into a dedicated
`~/devel/stitch-deploy` worktree (never the user's checkout), builds with
`--locked`, stages, upgrades itself as above, and reports its progress as
records (they sync). Order: **canary first** (the node where the change was
built and tested, normally tower), then mini, then laptop, then the phone; the
next node only after the previous is healthy for a few minutes; one failure
stops the rollout. mini's timer becomes `stitch fleet follow main` (pull,
build, stage, `node upgrade`), replacing the private script.

**The phone.** Only a Mac with Xcode can build and install it. Short term:
laptop runs `just ios device && just phone` as the phone's step when the phone
is reachable by `devicectl` (else the step waits and says so); the app's own
node then resumes suspended chats. Longer term: TestFlight (archive + upload
from the Mac, the phone updates itself; slower, but needs no cable or
proximity). Rollback on the phone is reinstalling the previous build (kept
in DerivedData) or TestFlight's previous build.

### 2.2 Layer 2: harness issues

A built-in tool any thread may call, `report_issue {title, what_happened,
expected, evidence?, area?}`, and `stitch issue` on the CLI. It writes an
`issue` record (it syncs, shows on the attention list, refers to the thread
and its step, so the fixer can read the transcript) and, when the node has
`gh`, mirrors it as a GitHub issue labelled `harness`. Cheap, never asks,
rate-limited per thread (as `notify`), and the tool description tells agents
to file and carry on, not to fix in the middle of the user's task.

### 2.3 Layer 3: the self-improve loop and tool

**Where it plugs in.** Not an Otter tool: Otter tools cannot start shells or
threads (`Tool.Call` refuses `spawn`, `wake_at`, `schedule`; there are no
process shards). The parts are:

- a **built-in skill `stitch-dev`** (compiled in, like `apps`): the
  procedure below, so any thread working on Stitch follows it;
- a **built-in tool `self_improve`** (Rust, small): it does not change code
  itself; it starts a detached *fixer* thread with a fixed goal template,
  place = the Stitch checkout, the skill on, a budget, and returns its ID.
  It exists so agents in any place can ask for a fix in one call, with the
  safety settings fixed rather than written by each model:

  ```
  self_improve {
    issue: "<description or issue record ID>",
    land: "pr" | "main",          # default pr
    rollout: "none" | "canary" | "fleet",   # default canary
    budget_usd?, model?
  }
  ```
- a **schedule** `harness-issues` (opt-in): an Otter `check` that returns
  the open `issue` records with no fixer (no model cost when none), and
  wakes a triage thread that groups duplicates and calls `self_improve` for
  those worth fixing, within a daily cap;
- layer 1's `upgrade_nodes {commit, nodes, mode}` as a built-in tool for
  the rollout step (asks; see safety).

**The fixer's procedure** (the skill):

1. `just worktree fix-<slug>` (its own branch and `target/`), reproduce the
   issue, ideally as a failing test first.
2. Fix; `just quick`; for a change to crates the app links, `just
   check-apple` (on laptop, via a thread there); for app UI, `just ui-shot`.
3. Commit, `git push -u origin fix-<slug>` (agents may push branches), wait
   for CI on devbox-1 (`gh run watch`), open a PR (`gh pr create`) with the
   issue, the cause, the test and the risk.
4. Land: `land: "pr"` stops here and asks the user (a question record: "merge
   fix-<slug> into main?", with the diff summary). `land: "main"` merges
   only when CI passed, the diff is under a size limit (e.g. 300 lines, no
   protocol bump, no migration, no change to approvals/sandbox/guard code)
   and the user's policy allows it.
5. Roll out: `upgrade_nodes {commit: <main sha>, mode: canary}` → tower
   (or the node it runs on) upgrades with health check; after probation,
   `fleet` continues to mini, laptop, iphone if asked. The fixer is woken with
   each result and reports. `just worktree-rm` at the end.
6. A **try on canary** mode for a branch: `upgrade_nodes {branch, nodes:
   [tower], probation_minutes: 60}` runs a feature branch on one node and
   rolls back to main's build automatically at the end unless kept.

**Safety summary.**

- *Approvals*: filing issues and starting a fixer do not ask (budgeted).
  Landing on main and every upgrade ask the user, unless `[self_improve]`
  allows them (auto-land small diffs; auto-upgrade the canary). The fixer
  never edits approval rules, the sandbox, guards, keys or config itself:
  those diffs always need the user (a path list checked by the tool).
- *Branch vs main*: fixes on branches; main only through the gate (CI green
  + `just quick` + approval/policy); the fleet only runs main's builds, but
  a canary may try a branch with automatic rollback.
- *Tests gate*: `just quick` locally, CI on devbox-1 on the pushed branch,
  `just check-apple`/`merge-check` on laptop when the app is touched; the new
  binary's `--selftest` before any restart.
- *Staged rollout*: canary → mini → laptop → iphone, healthy between steps,
  stop on first failure.
- *Health check and rollback*: §2.1 steps 4–5, by a watcher outside the new
  node; the previous build kept.
- *The upgrading thread survives*: the tool returns at once, the thread
  suspends at a step boundary and is woken with the result after the
  restart; restarts are done by the node (or its supervisor), never by a
  sandboxed shell.
- *Budgets*: each fixer within `budget_usd`; the schedule within a daily cap.

## 3. First steps (smallest first)

| # | Step | Effort |
|---|---|---|
| 1 | `BUILD` (commit, dirty, otter/engram commits) in `stitch --version` and `node status`. **Built** (`crates/stitch-cli/build.rs`, `stitch_node::build`, `Reply::Status { build, pending }`, local-45); peers' builds not yet | ½ day |
| 2 | `just restart` / `just update` fixed: restart when the running build differs (not only the protocol); `stitch node restart` that the node runs itself, so a thread's sandbox is never inherited. **Built**: the node watches its binary (10 s) and restarts into another build once idle, after checking it runs (`--version`); `stitch node restart [--now]`, `stop --when-idle`; the successor is started by the node's process (`node --after <pid>`: it waits while that process holds the lock; its own process group, with stdin from /dev/null and output to `node.log`, rather than the `setsid` of §2.1: it never had a terminal) or, under its service (decided once at the start, then the variables removed from the environment), the process exits 75 for it; a newer client restarts the node only into a build installed where its binary is (`tests/restart.rs`). Incident 2026-11-02 (tower ran its old build 9 hours after an install): the service's node waited 30 minutes for the lock behind a node a client had started outside it, a new build was installed meanwhile, and the node took the binary it found once it had the lock for its own (the watch only reacted to a change after that) and, its file renamed over, `current_exe` said `… (deleted)`, a path to nothing; the restart notice went to the other process. Fixed: the path is taken without ` (deleted)` (`upgrade::installed_exe`); the build a node runs is hashed from its own image (`upgrade::own_image`: Linux's `/proc/self/exe`; macOS, which has no such path and where a debug build takes seconds before its first instruction, so no early open wins the race, finds the file by the device and inode its code is mapped from, `proc_pidinfo(PROC_PIDREGIONPATHINFO)`: `current_exe` when that is still it, else the kept build that is a hard link to it, which the node before it made; a build found nowhere is taken to be the one installed, as before, and says so); and the watch compares the build installed with the running one at its first looks (20 s), not only after a change, so a node finds a newer build by itself whoever was told. A new build's restart is still never cut for threads at work, but one waiting past `STITCH_RESTART_NOTICE_SECS` (60 minutes) is told to the user once (a notice: which build, what keeps the node busy, `stitch node restart --now`), and `stitch node status` / `node builds` say `waiting since …` with what keeps it busy (`tests/restart.rs` `a_node_started_before_an_install_restarts_into_it`, `tests/root.rs` `a_restart_waiting_long_is_told_to_the_user_once`) | ½ day |
| 3 | Bring mini's `stitch-self-update` and units into the repo (`deploy/node/`), `stitch node service install` for systemd and launchd. **Built** (`crates/stitch-cli/src/service.rs`, `deploy/node/`, `<data>/service` written last so clients start the service and wait for its node (`STITCH_SERVICE_WAIT_SECS`, 30 s), even when starting it failed (it may be restarting on its own), and only a service still down past that gets a node of the client's own, said in `node.log` (since 2026-11-03: a client's node outside the service had held the lock and taken the new build's notice, incident above; `tests/restart.rs`); a running node is never cut: one outside stops once idle, the unit's own restarts into the new unit (`MainPID`, so mini's unit counts); the LaunchAgent has a label of its own, `dev.stitch.service.<path>`, and is never booted out under a running node; `deploy/node/install.sh` (`just install` and the self-update) stages, signs and renames into place; the self-update only from clean checkouts on main, and "changed" is the installed build not being their HEADs). Review fixes 2026-10-28; systemd in `tests/service.rs` (CI), launchd tried on laptop | ½–1 day |
| 4 | Builds kept: each build a node runs is copied (its running image, synced, checked against its hash, with its helper, build and store format) into `<data>/bin/<commit>-<hash>/` at its start; a build is known by the hash of its binary (the commits are its label: an Otter-only change or two dirty builds of one commit are two builds); the healthy one, the one before, the one on trial and two more are kept; build outputs (`target/`) are not. **Built** (`crates/stitch-cli/src/upgrade.rs`, `noderun.rs`). Simpler than the staging above: the binary stays where it is installed, and a rollback installs the kept copy back over it | 1 day |
| 5 | Drain. **Built, simple form**: the lock is taken before anything else at a start, and `<data>/restarting` holds clients back while a node restarts, so none starts a node of its own in the gap; a stop or restart someone asked for that waits for idle waits at most 30 minutes (`STITCH_DRAIN_SECS`), then stops what runs (detached threads resume where they were); a restart nobody asked for (a new build, a newer client) waits for idle, except that commands in shells (a server never ends) hold a new build's restart at most that long too, then are cut. Not built: deferring new turns while draining, suspending turns at a step boundary | 1–2 days |
| 6 | Health + rollback. **Built**: a build new here is on trial until it ran 60 s and answers on its socket (`<data>/bin/state.json`), and gives up (exit 70) when it does not answer 2 minutes later; under a service, a build that started 3 times without getting healthy is replaced by the healthy build (installed back, exit 75), starts ended by a stop or restart asked for not counting; without one, the old process watches its successor and does the same when it ends with an error or a crash, or never takes the lock, but not when it was asked to stop (status 0, `<data>/stopping`); a build that failed is never restarted into again, and gives way at once when started anyway; the rollback is said in the log and as a notice; never across `STORE_FORMAT`, which a test ties to the code of the store's files (`STORE_SHAPE`). `tests/restart.rs` (restart, a build failing and one hanging at its start rolled back, a stop during the trial not a failure), `tests/service.rs` (systemd, exit 75). Second review (2026-10-28): the trial's watchdog runs from before the node's start (a build hanging inside it while holding the lock gives up), and the watcher waits as long as the successor holds the lock; rollbacks only to builds kept whole, an install that fails running the current build instead of looping; a service's node asks a node outside it to stop once idle; `STORE_SHAPE` covers stitch-store's journal, root, blobs and change feed; SIGTERM during the start is a stop, not a failure; macOS install removes an agent of the old label. Not covered: under a service, a build that fails before its own start code runs | 1–2 days |
| 7 | `report_issue` tool and `issue` record kind. **Built** (`crates/stitch-node/src/issues.rs`, records only, §4, 3): an `issue` in the home workspace (title, area, node, build, place, a `thread` reference to the thread that filed it, the report as its body); its state is `set`s (`state`: open, fixing, fixed, dropped; `fixer`, `fix`, `note`); a live issue saying the same bug (its title's telling words mostly shared) is not filed twice; at most 5 an hour from a thread, 10 from a node, 30 in all, counted from the records (a restart forgets nothing); in the `weave` group, never asks; `stitch issue list|show|report` allowed by default, `set` the fixer's and the user's (`Request::Issue`, local-46) | ½–1 day |
| 8 | `stitch-dev` skill + `self_improve` tool. **Built** (`crates/stitch-node/src/selfimprove.rs`, the built-in skill `stitch-dev`). **Settings** synced (§4, 6): `self-improve` records, `stitch issue settings`, `[self_improve]` as a node's fallback; off until set. **Guarded:** a tool group of its own, off by default (on by its name only, never by `+*`), never handed on by `spawn`, always off for a client's end users (the Discord bridge); refused while off or no fixer node is set (never a fixer on the calling node), and on a fixer node whose update timer follows another branch than main (the canary follows main; only the unit's `STITCH_FOLLOW` setting counts, not its comments, and only the unit whose `STITCH_DATA` is the node's own data directory) and for read-only threads; it asks the user, whatever yolo says, unless the harness-issues schedule's thread (which has it on) or a chat the user started calls it, and always past an issue's 2 attempts; `stitch issue fix` from a thread's shell only for such a thread. **The ledger** (on the fixer node, under one lock, at its own start or its claim of another node's assignment, before the thread starts): the issue live, one live fixer at a time, 2 attempts per issue (reopened ones count) before the user, no new fixer while the last landed fix is not healthy on this node (the canary) for 30 minutes or was rolled back (the user past both; a fix commit the core's mirror no longer has, after a rewrite of origin's history such as the publication from a new root, counts as in the build), and the day's cap (UTC): a `fix-reservation` record holds the fixer's budget while it is live, then what its whole thread tree spent, plus the triage's own turns; the issue goes `fixing` with its fixer under the same lock, released (and open again, with why) when the thread does not start. A fixer the triage starts has its own budget, not the firing's. **The goal** quotes the issue as filed between markers the text cannot carry, as data, never instructions. Each landing (`fixed`) is a notice to the user. **The skill:** worktree, failing test, `just quick` (+ `check-apple` through laptop), CI, landing only what CI saw (a fast-forward of an unchanged main), protected paths landed only once the user approved the diff the node shows, the rollout left to the timers and watched. Not callable from `Tool.Call` | 1 day |
| 9 | The rollout, canary first (§4, 2). **Built, without a node protocol of its own**: the update timers carry it, each from a branch the node before promotes: tower follows `main` and promotes `fleet` (`stitch node service install --update --promote fleet`), mini follows `fleet` and promotes `fleet-mac` (`--follow fleet --promote fleet-mac`), laptop follows `fleet-mac` and uploads the app to TestFlight (`--follow fleet-mac --testflight <app checkout>`: the app is its own repository, built against this checkout; the self-update holds that checkout to the same rules, promotes it with Stitch, and runs its justfile by path, `just -f "$STITCH_APP/justfile" ios device` and `… testflight`, with `STITCH_DIR` this checkout; a plain `--testflight` uses the checkout's own justfile when it holds the app, and otherwise uploads nothing and says so). The self-update builds only clean checkouts on main holding exactly `origin/<branch>` (ahead or diverged: said, nothing built; and not while `just dev-link` is active); promotion pushes that exact Stitch commit (its `Cargo.lock` pins Otter and engram; until 2026-11-21 the timers followed and promoted `../otter` and `../engram` checkouts too) once the build installed is theirs and the running process has been up healthy for the whole trial (`stitch node builds --healthy-for 1800`: the process's time up, from `/proc` on Linux and `ps` elsewhere, the build it runs written at its start, so a crash or a restart begins the trial again); the canary finishes a build's trial (and promotes that build's recorded commits) before taking a newer main; the app checkout moving alone begins a trial of its own, with no build (the trial file, `~/.cache/stitch-self-update.trial`: `label=`, `at=`, `core=`, `app=`, written whole and moved into place, read by key; the older format, by line, is still read, from its time's line on, 2026-11-24, after a canary's file was found with four lines of other output before its own; a trial passes when the process ran healthy for it and it began 30 minutes ago, the latter what an app-only trial waits on), for at most `STITCH_TRIAL_HOLD` (2 hours), so a main that moves every few minutes cannot keep `fleet` from moving; a build that failed is not tried again until a commit moves; TestFlight is tried at each run until it uploaded that pair of commits, Stitch's and the app's (a stamp in the data directory). A checkout worked in (a branch, changes, commits of its own) pauses it silently, so after `STITCH_STUCK_AFTER` (3 hours) of skipping while origin's branch has a Stitch commit not installed, the user is told once (`stitch notify`), again only after a run got past the checks (2026-11-01, after laptop held TestFlight back for 18 hours); and each run past the checks refreshes the installed `stitch-self-update` from the checkout, which only `service install` wrote before. `tests/self_update.rs`. Not built: `upgrade_nodes` over the node protocol, an `[upgrade]` policy, a deploy worktree (the timers use the node's checkout) | 2–3 days |
| 9a | The phone through TestFlight: **done** (the app's repository: `just testflight` archives and uploads, recording the app's commit, Stitch's and the build number together; the first build uploaded 2026-10-28); external testers through Beta App Review after each upload, at most one submission a day (2026-10-30) | ½ day |
| 10 | `harness-issues` schedule. **Built**: on the fixer node, `stitch issue schedule` makes it from the settings (`every`, default every 6 hours, UTC; its daily cap `daily_usd`, its turns counted in the fixes' day), its check the node's own `@stitch:open-issues` (an Otter check cannot read records; `@stitch:` names checked when a schedule is made): only what changed since it last woke (new or changed open issues, fixers that ended with their issue still `fixing`) wakes its thread, never while self-improvement is off or the day's cap is spent; its thread triages (duplicates and non-bugs dropped with a note) and calls `self_improve` for one issue at a time | ½ day |

Steps 1–2 alone remove the most common surprise (an update that does not
take). Until step 9, the fleet step can be done with what exists: `spawn`
on each node with the goal "upgrade to <sha> with `just update` and report
`stitch node status`" (a quick model, a few calls each).

## 4. Decisions (2026-10-28, with the user)

The open questions of the proposal, answered:

1. **Landing:** a fix merges into main by itself once CI is green and
   `just quick` passes (plus `just check-apple` when it touches crates the
   iOS app links), as AGENTS.md asks of every change to main (the latest run
   of each workflow on the commit decides, so a failed run is superseded by a
   later green one: a rerun, or the same commit pushed as `fix-<id>-<name>`). No paths are
   reserved to the user: the `land: "pr"` default and the protected-path list
   of §2.3 are dropped; `self_improve` lands on main. *Revised after the
   review of steps 7–10 (2026-10-28):* protected paths are back for
   unattended landing. *Revised again the same day, with the user*
   (signed commits were too much: "a regular approve notification is
   enough, with a TL;DR of what needs to be approved"): the only gate is
   an **approval question at landing**. `stitch issue land <id> <commit>
   --summary "…"` (the node fast-forwards main for its fixer) asks the
   user before it pushes when the diff touches a path of
   `tools/protected-paths` (read from main as it is, never from the fix):
   the approval and allow rules and the fixers' mode (`rules.rs`,
   `fixer.rs`), caller identity (`peer.rs`), the sandbox (both
   `sandbox.rs`), self-improvement and issues (`selfimprove.rs`,
   `issues.rs`), the `stitch-dev` skill, `deploy/`, `.github/`, the list.
   The question is an ordinary `question` record, pushed to the phone and
   answered from the attention list or the app; yolo never answers it.
   **Its facts are the node's:** the TL;DR (the issue's title as filed,
   the protected files and the change's stat, from git) and then the
   protected files' **diff in full** (past 12 KB, cut, the whole diff
   attached as a file), a stat of the other files; what the fixer says
   (`--summary`, and its commit subjects) is shown apart, labelled as its
   own unchecked words and quoted. The app shows it monospaced and
   scrollable (Read in full, Whole diff), the notification its TL;DR, the
   TUI prints it with `v`. **Only the user answers it** (`user_only`): the
   node takes an answer on its socket only from a process no thread's
   shell started (ancestry, `peer.rs`) and that runs in no sandbox
   (macOS `sandbox_check`, Linux `NoNewPrivs`, which a sandboxed shell's
   commands carry however they detach, `setsid` and double forks
   included), and an allowing answer only if the user wrote it. A fixer's
   shells are always sandboxed (with no sandbox, each of its commands
   asks) and write nothing of the node's data directory. Declined, the
   branch stays and the issue is `open` again with the reason; allowed,
   the push only lands if main has not moved since (the user saw that
   diff). The signature gate (`tools/approvers`, `approved/<sha>` tags,
   CI's check, the self-update's check and `--approve`, the Otter and
   engram lists) is removed. Ordinary work by the user's own threads is
   not gated, as before. `tests/land.rs`. *Revised 2026-11-14* (design §2.28, roles), after
   the user approved every landing unread: before asking, the node puts
   the protected diff to the hazard guard's landing role (six questions:
   widens what agents do unasked, the sandbox, caller identity, the
   fixers' mode, the protected list or the gate, CI and deploy); with the
   auto-land on (`stitch issue settings --auto-land on`) and every answer under 0.3,
   it lands without a question, the user told in a notice with a
   one-line summary and `stitch issue revert <id>`; anything flagged,
   too large or unanswered asks as above, the verdict and the flagged
   questions first. The answering rules are unchanged.
2. **Rollout:** automatic, canary first. Once landed, tower upgrades; after
   its **trial period of 30 minutes** (healthy at start, no crash or
   rollback, its detached threads resumed), the fleet follows (mini, laptop,
   then the phone through TestFlight), each healthy before the next.
3. **Issues:** Stitch records only (an `issue` kind); no GitHub issues.
4. **The phone:** TestFlight, so it updates without the Mac at hand (the
   Mac archives and uploads from the app's checkout; the account steps are
   the user's, listed with the app).
5. **Supervisors:** systemd user units on tower (as on mini) and a
   LaunchAgent on laptop, installed by `stitch node service install`.
6. **Who fixes:** a designated **fixer node**, set in `config.toml`
   (`[self_improve] node`, `budget_usd` per fix, `daily_usd`, and the
   cadence, `every`); the `harness-issues` schedule runs there. *Revised
   the same day:* the settings are the user's and synced fleet-wide, as
   `model-choice` and variables are: `self-improve` records (the fixer
   node, its checkout, `budget_usd`, `daily_usd`, `every`, the model, on or
   off), the latest of the user's standing, set with `stitch issue settings
   --node <n> --budget <usd> --daily <usd> --on|--off` (asks when an agent
   runs it); `[self_improve]` stays only as a node's fallback. The day is
   UTC.
7. **Rollback:** simple: the previous build is kept and comes back when the
   new one fails its health check; a build that changed the store's format
   is never rolled back automatically: the rollout stops and the user is
   asked.
8. **The fixer supervises** (2026-10-31, asked by the user): the fixer
   thread reads the issue and the filing thread's transcript, decides the
   plan and makes the calls (what to change, whether a change is right,
   whether to land, the rollout), on its own model as before; the
   hands-on work (investigating, the failing test, the fix, `just quick`,
   watching CI, rebasing) is done by child threads it spawns, with
   `model: "quick"` by default. A child gets the default, stronger model
   only for a very complex job: a subtle concurrency, ordering or protocol
   bug (locks, async turns, sync, restarts, the store), a change across
   several crates or to the API or the store's format, a cause a quick
   child could not find, or a quick child that failed or produced a wrong
   or incomplete fix. The fixer stays light: it edits no code, reads its
   children's reports and diffs, checks them and decides. Its goal
   (`selfimprove::goal`) and the `stitch-dev` skill's "Supervise" say so.
   The fixer makes the worktree; its children (in the fixers' mode, as
   before) now write it too: a thread's own worktree is that of the issue
   fixed by the nearest fixer above it (`selfimprove::own_issue`), not
   only the fixer's own. Pushing, landing and setting the issue stay the
   fixer's alone; the children spend from its budget (its whole tree, as
   the ledger already counted).

## 5. Accepted risks (2026-10-28, with the user)

- **Promotion uses the node's normal git credentials.** No deploy keys,
  no separate Unix user, no GitHub rulesets (they need GitHub Pro for the
  private repositories). Any agent running as the node's user could push
  with those credentials, as it could before self-improvement existed;
  other threads ask for pushes unless the user granted them. Main is not
  protected against force pushes on GitHub. *Revised 2026-10-29, after a
  review of 49dce1e found three ways around the landing gate:* a fixer
  can no longer use those credentials, and the command line is no longer
  what holds it:
  - **It reads no credentials.** A fixer's sandbox (and that of a thread
    under it, and of the triage) keeps `~/.ssh`, `~/.config/gh` (and
    `$XDG_CONFIG_HOME/gh`, `$GH_CONFIG_DIR`), `~/.git-credentials`,
    `~/.config/git/credentials`, `~/.netrc` and on macOS
    `~/Library/Keychains` from being read (`Sandbox::unreadable`: Landlock
    read rules on Linux, granting every other entry from `/` down, so the
    directories holding those, `/`, `/home`, `~`, `~/.config`, are not
    listed; `deny file-read*` on macOS). Its shells lose `SSH_AUTH_SOCK`,
    `GH_TOKEN`, `GITHUB_TOKEN` and askpass helpers, and run git with
    `credential.helper` cleared and no prompt. `view_file` reads none of
    those paths for it; it reaches no SSH host and not the browser.
  - **The node does in git what needs credentials,** in a bare mirror of
    origin in its data directory (`<data>/self-improve/mirror.git`; one
    per repository since §6, `<data>/self-improve/<name>.git`, which
    no fixer writes), with hooks, fsmonitor and `ext::` off: landing
    (`stitch issue land`), pushing the fixer's branches (`stitch issue
    push <id>`: the fixer's command bundles its branch, `git bundle
    create`; the node copies the file, checks every head is the issue's
    `fix-<last part of its ID>` or `fix-<…>-<name>`, fetches it into the
    mirror and pushes it, forced), reading CI (`stitch issue ci`: each run's workflow, ID, URL and state, and for a failed run its failed jobs, steps and log tail) and
    fetching origin for the fixer to fetch from (`stitch issue fetch`).
    **Git never runs in the fixer's checkout for the node,** whose `.git`
    (config, hooks) the fixer writes. Origin's URL is read once, when the
    mirror is made at the first admission, from the checkout's
    `.git/config` as a file (`git config --file`, which runs nothing), or
    from `[self_improve] remote` in config.toml, which wins.
  - **The node serves a fixer's commands only what is a fixer's**
    (`fixer::refuses`), told from the connecting process, not from what it
    says: `stitch issue`, `stitch send` to its own issue's threads,
    notices, status, jobs, search, sync. No chat, spawn, answer,
    approval, yolo, tool or skill sets, schedules, keys, secrets, restarts.
    A sandboxed process that is no thread's shell (a fixer's command
    detached with `setsid`) gets the same while self-improvement is on.
    The command-line rules (`fixer_guarded`) stay as the fixer's guide.
  - **No other thread acts for it.** `send` from a thread in the fixers'
    mode reaches only its own issue's fixers (and the threads under them)
    and the triage thread that started it; the triage reaches only the
    fixers it started. The harness-issues schedule's triage thread, which
    a fixer's report wakes, runs in the fixers' mode itself (sandboxed,
    never yolo).
  - **What its sandbox writes no longer runs outside it** through the
    toolchains: of `~/.cargo` only `registry` and `git` (not `bin`, where
    the `stitch` the service starts is installed, nor `config.toml`, whose
    `rustc-wrapper` the self-update's build would run), and not rustup's
    toolchains nor the homes of Go, npm, bun, Gradle, Maven or pnpm (their
    `bin`s are on the user's PATH).
  - **Its own worktree is made for it** (decided 2026-10-29, issue
    2026-10-29T120308Z-mini-3byv, after a fixer on mini could neither
    make `../stitch-fix-qsh7-unit-data` nor write it once made: Linux's
    sandbox has no patterns, so only worktrees there at the turn's start
    are writable, and the directory holding the checkout never is). The
    node makes the empty directory `<checkout>-fix-<id>` beside the
    fixer's checkout and its sandbox writes it (`fixer::own_worktree`);
    the skill's `just worktree fix-<id>` has git fill it. No slug: the
    node cannot know it beforehand. `just`'s own runtime directory
    (`$XDG_RUNTIME_DIR/just`, where shebang recipes run) is writable to
    every sandbox, the rest of the runtime directory not.
  - **Still open:** the self-update pulls and builds in the checkout
    `~/devel/stitch`, which is also the fixers' default place: a fixer
    writes its `.git` (a hook, an fsmonitor, a filter would run in the
    self-update's `git pull`) and its `target/` (an artifact cargo takes
    as fresh). Until fixers work in a checkout of their own (a separate
    clone set as `place`, or the self-update building from the node's
    mirror), set `stitch issue settings --place` to a clone other than
    the one the update timer builds. The gates are wired in files beyond
    the protected list (`node.rs`, `tools.rs`, `weave.rs` call
    `fixer.rs`, `selfimprove.rs` and `mirror.rs`), so a fix could unwire
    them unasked; reviewing a landed fix stays the user's. On Linux a
    pathname socket of an SSH agent is still reachable if its path is
    known (Landlock does not confine `connect` before ABI 9's scoping).
  - **Asked rarely, and only when it matters** (2026-11-14, asked by
    the user, who answered all 114 of the fixers' and the triage's
    questions "allow" unread; classified in design §2.28). Deterministic
    first (`rules.rs`): a line is read with its substitutions, subshells,
    loops and redirections, and a here-document as data for the program
    reading it unless a shell does (any parenthesis, `$(` or `<<` used to
    ask); read-only `stitch` (`issue settings` alone, `schedule
    list|show|next`, `model ls`) and `systemctl` reading Stitch's own
    units (`status`, `show`, `cat`, `is-*` of `stitch*`, the listings) run;
    a fixer deletes its own `fix-<id>[-…]` branches and removes its own
    worktree (`Rules::own_issue`). Builds run inside: every sandbox writes
    `$XDG_RUNTIME_DIR/ccache-tmp` (ccache's temporary files; llama.cpp's
    build uses ccache) and opens `/dev/nvidia*` and `/dev/dri` (CUDA's
    `-arch=native` found no GPU in the sandbox), and the browser test
    starts Chrome with `--no-sandbox` inside one; the fixers had worked
    around both with `STITCH_LLAMA_GPU=off CCACHE_TEMPDIR=/tmp`. The
    user's own threads set issues' state and notes without asking
    (`set_asks`; a fixer or child setting another issue still asks). Then
    the hazard guard's fixers' role (`stitch issue settings --mode auto`): what
    would still ask, when it stays in the sandbox and only discards local
    work (`Rules::fixer_decidable`), runs if one decision call clears both
    the hazard and the authority question (0.5 each); flagged, unanswered
    or anything else asks as before, the verdict first. Kept: no
    credentials, no settings, approvals, yolo, keys or schedules, no
    writes to the node's data, pushes only through `stitch issue push`,
    services and timers only read; what needs to run outside the sandbox
    asks whatever the model says. Of the 114: 94 run, 1 is the guard's,
    19 ask (the user's settings, builds needing the iPhone or Xcode,
    nested sandboxes, the update service, a push to main).
- **What lands on main is installed.** The self-update builds whatever
  clean `origin/<branch>` holds; the gate is at landing (a fixer's
  protected change asks the user), not at install. A change pushed to main
  by other means is trusted as the user's.
- **An unsandboxed command passes for the user.** Answers to a landing's
  question are refused from sandboxed processes and from what a thread's
  shell started; a command the user let run outside the sandbox (yolo, an
  approved `outside` command) and then detached could still answer. A
  fixer is never yolo and is always sandboxed (or asked per command). Since 2026-10-29 the node serves a fixer's commands no answer at all,
  to any question (its own commands' approvals included), and none to a
  sandboxed process that is no thread's shell while self-improvement is
  on.

## 6. Two repositories (2026-11-22)

A second repository may build against this one: an app, say, with its
own origin, CI and recipes, which pins the core commit it was checked
with (a pin file such as `core.pin`, moved by its `just core-bump`). The
self-improve loop works with both once it is configured as a place; with
one repository it is as before. Below, "the app" is that second
repository.

**Places.** `places = [{ name, path, remote, core, pin, areas }, …]`
(`selfimprove::places`): each a repository fixes go to, its checkout on
the fixer node, origin's URL (unset: read from the checkout once), the
repository it builds against and pins (`core`, `pin`), and the `area`
words that send an issue there. The core is the one named `stitch`, else
the first: the nodes run its build. They are set in the synced settings,
`stitch issue settings --places '[{ name = "stitch" }, { name = "app",
path = "~/devel/app", core = "stitch", pin = "core.pin", areas = ["ios",
"app"] }]'` (a TOML array of tables; `""` clears them), which win over a
node's own `[self_improve] places` in config.toml. Unset: `stitch` alone
at the settings' `place` (`~/devel/stitch`); no second repository is
guessed from a directory beside it. `place` (synced, `stitch issue
settings --place`) always names the core's checkout. `stitch issue
settings` lists the repositories. A node whose build predates synced
`places` ignores them and rewrites the settings without them when it
changes one: set them (and change the settings) from an updated node.

**Mirrors.** One per repository, `<data>/self-improve/<name>.git`
(`mirror::ready`), each with its origin, hardened as before. The node's
one mirror of before, `mirror.git`, is the core's: renamed to
`stitch.git` the first time it is needed. A name is letters, digits,
`-`, `_`, `.` (never `mirror`).

**Issues.** `Issue.repo`: the issue's own (a field at filing or a `set`;
none means the core, as every issue before). Set by the triage or a
fixer (`stitch issue set <id> --repo <name>`), by `self_improve`'s
`repo` and `stitch issue fix --repo`, else decided when its fixer starts
(`route`) and set on it: a path it names that only some checkouts have
(the longest such prefix; `apps/ios/…` is only the app's, `crates/…` only
the core's), else its area's words (the place's `areas`), else the
core. The app's repository is
the safe guess: its fixer has both checkouts. The tool's description and
schema (`repo`, an enum of the names) come from the places; the triage's
check lists them.

**Fixers.** A fixer works in its issue's repository's checkout (its
place), with that repository's recipes (the same names in both: `just
worktree`, `quick`, `check`, `check-apple`). An app's fixer's sandbox also
writes the core's checkout (`sandbox::add_checkout`), and the node makes
the pair of empty worktree directories, `<app>-fix-<id>` and
`<core>-fix-<id>`, which the app's `just worktree fix-<id>` fills (an empty
directory there is filled, not taken for an existing pair). Its goal comes
from the places: where it works, the pair, how a fix in both lands, the
rollout by branches (`fleet`, `fleet-mac`, the Mac that uploads
TestFlight), never by node names; `check-apple` on a node whose system is
macOS.

**Commands.** `stitch issue push`, `ci` and `land` act on one of the
issue's repositories (its own and what it builds against): `--repo`, else
the commit's when only one mirror has it, else the one whose checkout or
worktree the command runs in (the CLI sends its directory: a hint; the
node checks what it pushes and lands), else the only one. A push refuses a
branch that shares no history with that repository's main (the core's
branch pushed from the app's worktree). `ci` reads that repository's runs
(`gh -R` from its mirror's origin). `fetch` fetches every mirror (or
`--repo`'s) and prints each one's command.

**Landing.** Per repository: a fast-forward of its main as it is, of a
commit CI passed on in that repository, protected paths read from that
repository's own main (`tools/protected-paths`, each its own list). A
landing in the issue's own repository sets it `fixed`; one in another of
its repositories (the core of an app's issue) is recorded and leaves it
`fixing`. Every landing is recorded on the issue (`landed`: `<repo> <from>
<to>` a line; the core's also as `landing`, which older builds read);
`stitch issue revert` takes them all back, newest first (the app's before
the core it pins), and clears them.

**A fix in both lands the core first.** The order is forced: a pin names a
core commit, and only a landed one may stay pinned on the app's main. So
the app's commit pins the core's fix commit itself, on the fix's branch:
the app's CI builds against it, and since a landing is a fast-forward of
that very commit, the pin stays right once it lands. `stitch issue land
<id> <app commit> --repo <app>` (`selfimprove::plan`) reads the
pin; when the core's main does not hold it, it must be on one of the
issue's own branches there with CI green, and the landing has two steps,
the core's first. Both are checked (fast-forward, CI) before anything is
pushed, and the user is asked once, over both diffs (`<repo>:<path>`), when
either touches a protected path. Then the pushes, in order, each with its
own lease.

Failure cases:
- *The app's CI fails* (the pin bump, or the app's change against the
  new core): nothing lands, the core's commit neither. Fix, push, land
  again.
- *The core's push fails* (its main moved): nothing landed; rebase the
  core's commit, re-pin, push both, land again.
- *The core landed, the app's push failed* (its main moved under the
  question): the issue stays `fixing` with the core's landing recorded and
  a note; the fixer rebases the app's commit keeping its pin, CI, lands it
  again: the pin is on the core's main now, so it lands alone. Meanwhile
  the core rolls out alone: the core's change must keep the app at its
  current pin building (add beside, never break), which the skill and the
  goal say. If it does not, the Mac's TestFlight run fails on the core
  without the app's half (3 tries an hour apart, then the user is told),
  until the app's commit lands.
- *The fixer lands the core alone* (`--repo stitch`, a staged fix): the
  issue stays `fixing`; the app's commit pinning it lands later, alone. A
  fix that needed only the core: `stitch issue set <id> --repo stitch`.
- *The fixer ends between the two*: the issue stays `fixing`, so the
  triage sees it (a fixer that ended with its issue fixing), with the
  note saying what landed; the next fixer finishes it.
`tests/land.rs` (`a_fix_in_both_repositories_lands_the_core_first`, on
local bare origins) runs the one-command landing with the app's CI
failing first, the user's revert of both, and the staged order with the
app's CI failing after the core landed.

**Rollout.** A core commit as before. An app commit runs in no node's
build: the update timers carry the app's checkout beside the core's
(`stitch node service install --update … --app <checkout>`: `STITCH_APP`,
following the same branch as the core) and promote it with the core, to
`fleet` and `fleet-mac` on the app's origin; the Mac uploads TestFlight
from its app checkout (`--testflight <checkout>`) once it holds the new
pair. An app commit landed alone (the core's build already up to date)
begins a trial of its own on each promoting node (2026-11-24): pulled, no
build, the trial file rewritten with the new app commit and a new start,
the core's build kept; promoted once the node ran healthy for the trial and
the trial began 30 minutes ago; a trial still running (its commits not
promoted) is finished first, as the canary does with a newer main. Before
that, the trial was written only at an install, so an app commit landed
after the core's install was pulled but never promoted. The Mac uploads
the new pair once (its stamp holds the pair). So the canary's trial (the next fixer held until the last landed fix
ran healthy 30 minutes there, `last_fix_pending`) is about the core's last
landing only (`last_core_fix`): an app-only fix holds no fixer back. An
app's issue is refused on a fixer node whose update unit (for its own
data directory) does not carry that app checkout, or follows another
branch than main for it (`not_canary`): its fix would reach no node.
