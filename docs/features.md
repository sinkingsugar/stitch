# Stitch feature guide

Detailed behavior and configuration for Stitch's implemented features. For a
first installation, start with the [README](../README.md) and [setup manual](setup.md).
For command syntax, see the [CLI reference](cli.md) and [TUI reference](tui.md). Architecture,
future work and decision history live in [design.md](design.md); the record
format is in [records.md](records.md).

## Contents

- [Crates](#crates) and [providers](#providers)
- [Interactive shells and terminal UI](#the-root-agents-shell)
- [Files and other models](#seeing-files-and-other-models)
- [Apps and the dashboard](#apps)
- [Memory](#memory)
- [Isolated web tools](#the-web) and [your own Chrome](#browser-control-your-own-chrome)
- [MCP servers](#tools-from-mcp-servers) and [Otter tools](#tools-written-in-otter)
- [Per-thread tools and skills](#each-threads-tools-and-skills)
- [Projects and folders](#where-threads-belong)
- [Delegation and read-only threads](#threads-that-start-threads)
- [Schedules and wake-ups](#schedules-and-wake-ups)
- [Secrets, variables and SSH](#secrets-variables-and-ssh)
- [Loose ends, deletion and notifications](#loose-ends-deletion-and-notifications)
- [Integrations and experimental features](#integrations-and-experimental-features)

## Architecture at a glance

- **Store:** folders of immutable records in the node's data directory, with indexes derived from them; blobs by content hash.
- **Sync:** record sync between any two nodes (phones included) over iroh; a live channel for streams and nudges later.
- **Agents and tools:** the core is Rust, and so are the root agent's loop and tools for now; they move to [Otter](https://github.com/sinkingsugar/otter), the language that succeeds Shards 1.x, checked before they run.

## Crates

- `stitch-records`: the record format. IDs, references, records (Markdown with TOML front matter) and transcript segments (JSON Lines).
- `stitch-store`: record roots on disk, the blob store, the SQLite index with derived status and the graph's edges, record sync (range-based reconciliation).
- `stitch-vault`: experimental E2EE vault protocol and inert recovery client, separate from node sync, driven by `stitch vault` ([protocol](vault-protocol.md)); the hosted service is Fragcolor's, deployed for testing only; not production-ready.
- `stitch-net`: the protocol between nodes over iroh: record sync, blob fetch, pairing, the node key and the trust list.
- `stitch-providers`: model providers by hand over HTTP and server-sent events: Anthropic, OpenAI-compatible Chat Completions, Responses (OpenAI/Codex, xAI), and Gemini's native API (audio and video in, images out).
- `stitch-shell`: interactive shell sessions on PTYs: exact command state, a line log and a terminal screen.
- `stitch-node`: the node: config, credentials, the root agent's loop and its shell tools, the local client protocol, the sync loop.
- `stitch-ffi`: the node for Apple clients (the iOS and visionOS app, in its own repository), through UniFFI.
- `stitch-cli`: `stitch`: the node (`stitch node`) and its CLI; beside it `stitch-shell-helper`, which forks shells for the node on macOS and is installed next to `stitch`.
- `stitch-llama`: llama.cpp in-process (a submodule, built with CMake; Metal on Apple, iOS and visionOS included): embeddings of text and images.
- `stitch-memory`: memory, engram over EmbeddingGemma 2, as a fold of synced facts.
- `stitch-proxy`: `stitch proxy`, a key-holding proxy for model APIs: clients get tokens with daily budgets and rates instead of the key, usage metered from the (streamed) responses.
- `stitch-discord`: `stitch discord`, a Discord bot in front of a node: mentions and `/ask` start Stitch threads answered in Discord threads. With the proxy (`stitch proxy`) in front of the model keys, a node can serve a Discord server.

- `stitch-ssh`: persistent remote shell sessions, SSH host configuration,
  host-key trust and device-key authorization.

Building and installing require sibling Otter and engram checkouts as well as
the pinned llama.cpp submodule. See [setup §1](setup.md#1-install-update-restart)
for prerequisites, dependency-access limitations and installation, and
[setup §2](setup.md#2-pairing-peers-sync) for pairing.

The node runs on after the command that started it. When a newer `stitch`
finds an older node idle, that node stops and the newer one starts. If the
older node is busy (a chat or thread at work, a command running in a shell),
it stops once idle. `stitch node stop` stops it at once.

## Providers

Subscription sign-in follows edge talk's provider profiles:

```sh
stitch auth login anthropic      # Claude subscription
stitch auth login openai         # ChatGPT / Codex subscription
stitch auth login grok           # Grok subscription
stitch auth status
stitch chat "hello" -m openai/gpt-6-astra
stitch chat "hello" -m grok/grok-4.5
stitch auth logout openai
```

In the TUI, use `/login [provider] [--file]`, `/logout [provider]`, and `/auth`
(or `/auth <provider>`); without a provider they offer a list. The sign-in panel
is temporary and is never saved into the chat. Ctrl+T returns to the
conversation.

Commands that need something to pick offer a list you filter by typing: `/`
alone lists the commands, `/model` the models your providers offer (asked from
them, kept a few minutes; `/model provider/model` names one directly),
`/spawn` your machines and then their models, `/send` your threads, `/memory` on or off for the open thread (the
status line shows which), `/link` the nodes to link a folder with.

Open the printed URL in a browser. The command waits for a local callback;
for a remote machine, forward its callback port (Anthropic 53692, OpenAI 1455,
Grok 53694) to localhost before signing in. Use `auth login <provider> --file`
for owner-only token files on a headless machine ([setup §3](setup.md#3-providers)). Tokens refresh automatically,
stay local to this data directory, and never sync with conversations.

A subscription you signed in to always wins over an API key, stored or in the
environment; keys serve providers without one. `STITCH_<PROVIDER>_API_KEY`,
`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `XAI_API_KEY`, and `GEMINI_API_KEY` (or
`GOOGLE_API_KEY`) are supported. Gemini is built in like the others (`gemini/<model>`,
its models listed from Google's API; keys only, no sign-in): it reads audio and
video attachments, and its image models (`gemini/gemini-3.1-flash-image`, …)
answer with images, which arrive as the reply's attachments. With a Gemini key,
any model (Claude's, GPT's) can make and edit images too, with the
`generate_image` tool (`[images] model` picks the image model); its images come
in the reply the same way. In the app a reply's images show large in its
bubble; a tap opens them full screen to zoom, share, save, copy, mark up, or
send back with what to change. Sign-in works only with the built-in `anthropic`,
`openai`, and `grok` providers at their official endpoints; custom endpoints
never inherit a subscription token. Available models depend on your account.
These profiles mirror edge talk and can require updates when provider protocols
change; a failed subscription request never switches billing routes.

## The root agent's shell

Stitch works through interactive shell sessions on the node: it runs commands,
answers programs that ask something, and reads full-screen programs. On macOS
and Linux the shells run in a sandbox. They write only in the thread's
checkout (its worktrees too), temp and build caches, and only what reaches
beyond that asks: a push, a publish, an upload, throwing away uncommitted
work. Elsewhere, read-only and allowlisted commands run. Anything that asks
waits for your approval (`y`/`n` in
the TUI, a prompt in `stitch chat`, or from the attention list on any machine:
Ctrl+T, `stitch attention`; the TUI's status line shows `⚑ N` while N wait). `stitch mode yolo` (`stitch yolo on`, or `/yolo` in the TUI,
whose status line then says `yolo`) lets everything run without asking until
`stitch mode ask`; `stitch mode auto` (`/auto`) does too, except what the hazard
guard judges could do severe damage that cannot be undone, which asks (design
§2.28). It is saved as `approval` below. In `config.toml`:

Long commands run in the background: `shell_run` with `background` returns at
once, and the thread is told when the command ends, with what it printed. If
the thread is working, the note goes in after its current tool calls;
otherwise it wakes the thread. `shell_watch` does the same for a command
already running, or for the first line matching a pattern (a server's
"listening on"). `shell_wait` blocks until either happens. Reports from
spawned threads arrive the same way. `stitch jobs` (`/jobs` or Ctrl+B in the
TUI) lists what runs in the background: shell commands on this node and
detached threads on every node. `stitch jobs stop <id>` (`x` in the TUI) sends
Ctrl+C to a command, closes its session if stopped again, and cancels a
thread. Sessions and jobs live in the node and end when it stops. The agent
has the same in its tools: `jobs` lists its jobs (and those that ended in the
last hour), `job_stop` stops one for real (Ctrl+C, then SIGTERM, then SIGKILL
to the command's whole process group, so a build's compilers and test
binaries go too), `job_output` reads a job's output, live or ended, and
`max_secs` gives a job a time limit (`stop_on_timeout` stops it then).

Jobs also watch screens and files, at no model cost. `shell_watch` with
`screen`, `screen_gone` or `quiet_secs` fires when a full-screen program (such
as `claude`) shows something, stops showing it, or goes quiet; spinners and
timers don't count as change. `watch_file` fires when a file changes, gets a
matching line, or goes quiet. `shell_send` pastes text the way a terminal does
when the program asks for it, and `submit` presses Enter once the screen
settles. `stitch shell attach <thread>/<session>` puts a session in your
terminal: what you type goes in as yours, the agent is told, and it sends
nothing on top of it. Ctrl+] detaches.

```toml
[shell]
approval = "sandbox"    # the default where there is one; "ask": allowlist only;
                        # "allow": everything runs (yolo); "auto": yolo
                        # with the hazard guard; "off": no shell
allow = ["cargo test"]  # command prefixes added to the defaults
writable = []           # more directories sandboxed shells write
user_shell = false      # true: your own bash or zsh with your rc
```

More in [setup §4](setup.md#4-shell-approvals); what "the shell did not
start" messages mean is in [§11](setup.md#11-troubleshooting).

The shells a chat starts have the environment of the `stitch` that chats: your
terminal's `PATH`, SSH agent and credential helpers, wherever the node was
started from. Detached threads have the node's.

The root's instructions are `agents/root.md` in the home workspace when it
exists; a thread keeps the instructions it started with.

Threads are also told the instruction files that apply where they work:
`AGENTS.md` at the top of the home workspace (yours, on every machine), then
the project's or folder's `AGENTS.md` from its top down to where you are (a
directory without one has its `CLAUDE.md` read, with its `@file` imports).
Skills are `skills/<name>/SKILL.md` in the home workspace, with a `name` and
`description` header; threads see the list and read a skill when a task fits
it. When the files change, a running thread gets them with your next message.
Stitch has skills of its own (`apps`, `leadsheet`, `google-flights`,
`otter`), read with `read_skill`; one of yours of the same
name replaces it.
User-level files and skills of other agents (`~/.claude`, `~/.codex`) are not
read: they name tools Stitch does not have.

The TUI writes the conversation into the terminal's own scrollback as plain
text, so selecting, copying, scrolling and searching are the terminal's; only
the input, the reply while it streams and the status line stay live at the
bottom. Ctrl+O (or `/threads`) continues another thread, printing its
transcript; Ctrl+P (or `/parent`) goes up to the thread that spawned the open
one (the status line shows `↑ <parent>` on a child), `/children` lists the
threads it spawned to open one; Ctrl+N starts a new one;
Ctrl+Y (or `/copy`) copies the last reply; Ctrl+T shows what needs you. `/fork [N] [-a] [-t]` continues the open thread
in a new one (all of it or its last N messages, without attachments or tool
calls), `/summarize` from a summary of it, `/compact [N]` from a summary and
its last N messages; `stitch fork` does the same from the CLI. `/search`
(`stitch search`) finds messages in every thread by their words and meaning;
`/rename`, `/pin` and `/quote` name, pin and quote the open thread; new chats
are titled from their first exchange. A thread near
its model's context window continues in a new one, summarized, on its own
([setup §10b](setup.md#10b-threads-search-titles-pins-forks-summaries-and-compaction)). Shift+Enter starts a new line (where the terminal
tells it from Enter: Rio, kitty, WezTerm, Ghostty, iTerm2 with CSI u), as do
Alt+Enter, Ctrl+J, and `\` then Enter anywhere. Opening another thread or
starting a new one while a reply runs lets go of it: it goes on at the node,
its approvals on the attention list.
Esc Esc on an empty input picks one of the open thread's messages (↑↓, its
start shown below): Enter forks from it, with options, edits and resends one
of yours (a fork just before it, its text back in the input), or copies it.
`@` in the input picks a file of the folder (git's files, fuzzy): a text file
is attached while `@path` stays in the text; others are named for the agent.
Ctrl+V attaches the clipboard's image (or a file copied in Finder) as
`@image-1.png`, and a file dropped on the terminal is attached rather than
pasted as its path (an image, a PDF: any file that is not text).

While a reply runs, what you type is queued and given to it after its current
tool calls, written into the thread where it went; what it did not take is
sent when it ends. ↑ takes the last queued message back to edit. Esc stops the
reply: a running command gets Ctrl+C, and what was queued comes back to the
input.

## Seeing files, and other models

- `view_file` shows a file on the machine the way `browse_web` shows a
  download: images to the model, PDFs as text with page images, spreadsheets
  and CSV as tables. The shell can only show text; this is for screenshots,
  scans and charts. A directory comes back as a listing (sorted, directories
  ending in `/`, files with their size, symlinks marked; at most 1000 entries,
  saying how many were left out; `depth` for more levels), so a node with no
  shell, like the phone, can look around too.
- Every image the model sees comes with an ID (`img:…`); `zoom_image` crops a
  region of it from the original at full resolution. Images are scaled down to
  what providers take when sent; the original stays in the conversation.
- Another model's view (a second opinion, another vendor, a cheaper model for
  bulk work) is a `spawn` with tools off; `wait` has the answer, and what it
  cost counts toward the thread's budget.

## Apps

Agents make apps: HTML, CSS and JavaScript (a dashboard, a calculator, a
form, a game, a document to download) written into a directory and shown
with `show_app`. Each call shows a version, frozen, in the conversation; the
node serves it, the same everywhere:

- on the phone, in the conversation (at most 600 points tall, full screen
  from its bar), with its downloads and printing through the system;
- on a computer, in the browser, at an address of its own under
  `localhost` (`show_app` gives it, and the agent can check the page with
  `browse_web`).

Pages keep state with `localStorage` (or `stitch.storage`), saved as records
of their conversation: the same on every device, changed live when another
writes. `stitch.report(data)` sends the agent something from the page, and a
page that fails as it opens tells the agent its first error, for a fix.
Edge talk's libraries come bundled at `/_lib/`, offline (Chart.js, D3,
Mermaid, Leaflet, KaTeX, Three.js, JSCAD with MeshRepair for 3D printing,
pdfmake, SheetJS, the leadsheet music stack, …); an app reaches the network
only when shown with `networking`. The agent's guides are the built-in
`apps` and `leadsheet` skills.

Pinned apps are on the phone's thread list and its home screen widget, and
in the browser:

```sh
stitch app pin <thread> --icon checklist         # the latest app a thread showed; follows it
stitch app pin blake3:…                          # a version, fixed: it stays there
stitch app list                                  # pinned apps and their addresses
stitch app open todo                             # in the browser; no name: all of them
stitch app open <thread>                         # the latest app a thread showed, pinned or not
stitch app unpin todo
stitch dashboard                                 # the built-in dashboard, live; pinned and opened
stitch app allow-live todo [--thread <id>] [--deny] # let an app read threads, jobs, approvals
```

The dashboard (also offered on the phone's thread list until pinned) shows
your threads on every node, what needs you and this node's jobs, live from
the node: no model involved. Any app can read the same through
`stitch.live`, once you allow it: the phone asks with a sheet of its own;
an app in a browser has no trusted way to ask, so allow it with `stitch app
allow-live`.

An app can also run **threads of its own** (design §2.22): a studio where
each project is a conversation with an agent beside a 3D view, say. It
declares what they need in a `stitch-app.json` next to its `index.html`
(places, tools, skills, Otter and MCP tools that run without asking, models,
budgets per thread and per day, how many at once), and you grant that once,
having seen the list:

```sh
stitch app allow-threads "Atelier" --thread <id>                 # shows what it asks, writes nothing
stitch app allow-threads "Atelier" --thread <id> --needs <hash>  # grants exactly that; --deny takes it back
stitch app allow-container "Atelier" --thread <id>               # its Containerfile and commands (design §2.23), writes nothing
stitch app allow-container "Atelier" --thread <id> --build <h> --run <h>  # grants them; the build starts
stitch app storage [--rm "Atelier"]                               # app images and caches on this node, or one app's removed
```

Its page then creates threads, talks to them and shows their replies
(`stitch.threads`), and sees only the threads it created. Every call of
theirs passes one check against the grant standing at that moment: only the
tools granted, files only in their place (instructions and settings there,
like `.stitch/` or `AGENTS.md`, and anything outside it still ask you, on
your devices, never in the app), the web only at public addresses, no
shell, never yolo, none of your own allowances; the money is counted at
every model call. Grant an app a folder of its own rather than a checkout
your own threads build: what it writes there would run when you build. A
changed manifest asks again; a denial stops its turns at work.
`examples/app-threads/` is a small one to try (`tools/app-threads/try.sh`
runs it against a test node).

An app whose threads must run programs (Blender, say) ships a
`Containerfile` and declares **commands** (design §2.23, Linux with rootless
podman): you grant the build (it runs the app's Containerfile with internet
access, shown with its change since the last grant) and the commands; each
then runs in a container of the app's image with no network, the image
read-only, as a user that owns nothing of yours, its arguments typed, its
inputs copied in, what it writes read back as untrusted into the app's
**data folder** (`~/Apps/<name>` by default: your files of that app, apart
from its code, which its page reads and writes with `stitch.files`).
Atelier's `rebuild_garment` is the first (`tools/app-containers/atelier_e2e.sh`
runs it against a test node).

A pinned app follows its conversation: when the agent shows a newer version
under the same name, the pin moves to it and keeps its data (pinned from the
phone, it follows too). One pinned at a version (`blake3:…`) stays there
until pinned again; pinned again from its thread, it follows once more. A
pinned app's `stitch.report` (a Refresh button, say) wakes the conversation
it was pinned from, with the same rate limit as in the chat.

The browser gets in with the link's token once (then a cookie of that app's
address); nothing else on the machine does. `[apps] port` in `config.toml`
moves the server (7177 by default), 0 turns it off.

## Memory

Stitch remembers what was said and seen across conversations, on every
machine. Install the model once per machine (EmbeddingGemma 2, about 310 MB,
and 555 MB more to remember images too):

```sh
stitch memory install            # or --text-only
stitch memory status
stitch memory off [thread]       # this thread out of memory (the latest here by default)
stitch memory on [thread]        # and back in
stitch memory audit              # decisions by novelty band, strengths: the drift check
stitch memory notes              # the data to recalibrate the note gate with (setup §10)
```

The policy is edge talk's, field-tested there and ported whole (design §2.9):

- **Remembering.** After each turn, the node that ran it embeds the turn's
  messages and images and records the facts in the thread's repository, so
  they sync like the conversation; every node folds the same facts into the
  same memory graph (engram). A near-duplicate strengthens what it repeats; a
  newer one carrying numbers (a dose, a price) becomes the version recalled.
- **Recalled unasked.** Before a turn, the message is the cue: memories it
  brings up, and what earlier messages surfaced, are checked against it (how
  close depends on how familiar the topic is) and the closest few join the
  message as a note, never twice in a thread. A message without a topic
  ("ok do it") gets no note. The `recall` tool searches on request: memory,
  then a keyword search over every thread.
- **Learning.** An answer that uses a memory strengthens it. One that
  dismisses its memories with an invisible marker (U+2060), or cites one
  without using it, knocks them; memories shown again and again without
  helping are knocked when memory sleeps, once every twenty hours across
  machines. A quiet cluster of related memories is condensed into a gist by
  a model.
- **Afterthoughts.** When an exchange brings up something new, the model may
  add one short afterthought, at most once per message of yours; it can
  decline invisibly.
- **Off for a thread.** `stitch memory off` (or `/memory off` in the TUI)
  keeps a thread out: it is not remembered and what it held is not recalled.
  `on` brings it back, remembering what it holds. A message containing
  U+2063 is never remembered.
- **Spawned threads recall, and are remembered only when asked.** A thread
  another thread starts (`spawn`, `stitch chat --detach`, `/spawn`) is not
  remembered unless started with `remember: true` (`--memory`), but it may
  search memory with `recall` unless started with `recall: false`.
  (`memory: true` still means both, `memory: false` neither.) Your own
  chats are remembered as before. What threads said among themselves, the
  user in none of their messages, comes after your own conversations when
  you search.

```toml
[memory]
enabled = true   # false: no memory on this machine
images = true    # false: text only (the vision projector stays unloaded)
gpu = true       # false: encode on the CPU
gist_model = "anthropic/claude-haiku-5-5"   # writes gists; quick when unset
```

## The web

The root has three web tools:

- `browse_web` reads a URL. Web pages render in a headless Chromium (on the
  phone, in the app's own WebKit, offscreen) with their JavaScript run, and come back as Markdown of their main content, links
  included; `wait_for` (a CSS selector) waits for content that appears late,
  and `screenshot` (or `screenshot_selector` for one element) adds a picture
  of the page.
  PDFs come back as text by page, with page images where a page has no text or
  large pictures (`start_page`/`max_pages` read long ones in ranges). Images
  come back as images the model sees, spreadsheets (XLSX, XLS, ODS) and CSV as
  Markdown tables, other files saved with their path for the shell.
- `web_eval` loads a page and runs the model's JavaScript in it (an async
  function body; `return` the result, console output comes back too): for
  structured data out of JavaScript-heavy pages.
- `search_web` searches with Brave (default) or Perplexity, once a key is
  stored: `stitch key set brave` (or `perplexity`, with `[search] provider =
  "perplexity"`; `BRAVE_API_KEY` / `PERPLEXITY_API_KEY` work too).

It is all Rust in the one binary: Chromium is driven over its DevTools
protocol, PDFs are read and rendered without Poppler, spreadsheets without
Office. The browser is the one thing outside: an installed Chrome, Chromium,
Edge or Brave is found on its own; on a machine without one (a server),
`stitch browser install headless` downloads the stable headless Chromium (about 100 MB)
into the data directory. Nothing is installed by a tool call.

The browser starts with the first web call, serves the calls after it, and
stops after five idle minutes. Each call gets a fresh browser context: no
profile, cookies or logins of yours, nothing shared between calls. Pages can
reach local services (it is your machine). Downloads go to `<data>/downloads/`
and are kept a week; images the model saw are kept in the conversation and
sync with it. Calls take up to 30 seconds by default (`timeout_secs`, at most
120); text is cut at 32,000 characters, the whole saved for the shell; files
up to 50 MiB.

```toml
[browser]
enabled = true             # false: no browse_web or web_eval
chrome = "/path/to/chrome" # default: installed by `stitch browser install headless`, else found
idle_minutes = 5

[search]
provider = "brave"         # or "perplexity"
```

## Browser control: your own Chrome

Agents can also work in **your own Chrome**, with your profile and logins,
beside you: through Stitch's extension, in a tab group of their own
(design §2.18, [browser guide](browser-tool.md)). Desktop browser control is
implemented; [its progress notes](browser-tool.md#16-b1-progress-2026-10-18-branch-browser-tool)
and design §2.18 track the remaining phases, including watching from the phone.

```sh
stitch browser install       # the extension's files, and the native host for Chrome, Chromium, Brave, Edge
```

Then, in each Chrome profile Stitch may use: `chrome://extensions`,
Developer mode on, **Load unpacked** with the folder it printed
(`~/.stitch/browser/extension`), then the Stitch icon in the toolbar and
**Allow Stitch in this profile** (give the profile a name there). Nothing is
reachable until you switch a profile on, and only there; `stitch browser
profiles` lists them (`disable <name>` switches one off). After installing
a newer Stitch, run `stitch browser install` again and press the reload
button on the extension's card.

- The `browser_*` tools appear for desktop threads while a profile is
  connected and enabled. The agent opens its own tabs, in the background of
  your window (cooperative) or, for detached threads, in a window of its own
  (agent-alone); it never sees your other tabs. Chrome shows "Stitch started
  debugging this browser" while it acts; Cancel there stops it.
- **You always win:** your typing, clicks or scrolling in an agent tab stop
  it there; it goes on 4 s after you stop, from a fresh read of what you
  changed. The page shows a glow, its cursor and **Stop**, **Take over** /
  **Hand back**.
- Going to another site, and paying, sending, deleting or account changes,
  always ask, yolo included. It never types passwords, codes or card
  numbers: it hands those to you.
- Questions show in the page while you watch (Allow once · Always on this
  site · Deny), else on your devices with a screenshot of the tab; "always"
  keeps the site for the thread's place.
- Point at things: right-click "Ask Stitch about this…", the popup's picker,
  or "Share this tab"; the agent sees them in `browser_context`.
- A thread that uses your browser leaves memory (`/memory on` brings it
  back).
- `stitch browser call <tool> '<json>'` runs a tool by hand.

```toml
[browser]
sites = ["github.com"]       # act there without asking, even unwatched
sensitive = ["mybank.com"]   # every change asks; nothing unwatched
blocked = ["example.org"]    # never opened
resume_after_secs = 4
```

## Tools from MCP servers

Threads can use the tools of MCP (Model Context Protocol) servers, next to
the built-in ones. Servers are this node's, in `config.toml`: a command
(stdio) or a URL (streamable HTTP). Nothing is offered until it is enabled,
for every thread or for threads working in a directory, so the tool list
stays small:

```toml
[mcp]
enable = ["fetch"]                  # every thread with tools gets these

[mcp.servers.fetch]                 # stdio: started by the node
command = "uvx"
args = ["mcp-server-fetch"]
allow = ["fetch"]                   # runs without asking

[mcp.servers.github]                # streamable HTTP
url = "https://api.githubcopilot.com/mcp/"
key = "github"                      # stitch key set github: sent as a bearer token
tools = ["get_*", "list_*", "search_*", "create_issue"]   # all it offers here

[mcp.servers.db]
command = "~/bin/db-mcp"
env = { DB_URL = "postgres://localhost/app" }
key = "db"                          # a stdio server gets its key in key_env
key_env = "DB_TOKEN"
timeout_secs = 300                  # a call's longest wait (default 120)
approval = "allow"                  # every call runs without asking

[mcp.projects."~/devel/app"]        # threads whose shells start here, or below
enable = ["github__get_*", "github__list_*", "db"]
```

The model sees them as `<server>__<tool>` (`github__get_issue`). An entry
in `enable` is a server (all it offers) or a pattern over those names.
Calls follow the shell's rules: what `allow` (or `approval = "allow"`)
does not cover waits for your approval, shown as
`run [mcp:<server>]: <tool> <arguments>`, unless yolo is on. A server starts
the first time a thread needs its tools, serves every thread on the node,
starts again after a crash, and stops with the node (with whatever it
started). Images in results reach the model; long text is cut at 32,000
characters, the whole saved under `<data>/downloads/`. Changes to the
configuration apply from the next turn. `stitch mcp` starts each server,
lists its tools (marking those enabled where you are) and stops it again.
Not supported yet: OAuth (static keys and headers only), resources and
prompts, sampling, and the old HTTP+SSE transport. Every key, full paths,
logs and common failures: [setup §5](setup.md#5-mcp-servers).

A server can serve your other machines too: with `share = true` in its
`[mcp.servers.<name>]`, threads elsewhere (the phone included) that enable it
list its tools as this machine's (`MCP server mail on mbp: …`), and their
calls run here, under this machine's `allow` and approvals (an approval is
asked as a question you can answer from any machine). When this machine
cannot be reached, a call fails at once saying so, and the thread can queue
it (`queue_call`): it runs once this machine syncs again, and its result
comes back to the thread as a report. A worked example, a Mail.app server
shared from the Mac: [setup §8](setup.md#8-worked-example-mail-with-mailapp-on-the-mac-and-from-other-nodes).

## Tools written in Otter

A skill can bring tools of its own: Otter scripts in its `tools/` folder
(`.otr`; `.otter` and `.shs` work too), compiled and type-checked by Otter
while the node runs, and offered to the model next to the built-in tools
while the skill is on, on every node, the phone included. Stitch's own
`google-flights` skill is one: `google_flights_search` and
`google_flights_cheap_dates` read the public Google Flights website, no key
needed.

Stitch's `otter` skill (on unless turned off, `-otter`) gives a thread the
language itself, on any node, the phone too (which has no shell for the
`otter` command): `otter_search` and `otter_describe` over the catalog,
`otter_check` to check code or a script without running it, and
`otter_run` to run it. They answer as `otter search`, `describe`, `check`
and `run` do. A run that only computes runs at once; one with effects asks
(`run [otter]: otter_run <code>`), as a tool does. So a thread can write a
skill's tool, check it and use it from the next turn.

```
~/.stitch/home/skills/units/SKILL.md          # a home workspace skill, as usual
~/.stitch/home/skills/units/tools/convert.otr # its tool
~/devel/app/.stitch/skills/deploy/tools/…     # a place's skill's tools
```

A script declares functions and ends with its definition, which names the
function a call runs:

```otter
@fn(Greet input: {input: {} env: {} working-dir: String} output: String params: {} {
  input.input.name | If({IsString} {ExpectString} {"nobody"}) = name
  f"Hello, {name}!"
})

{
  definition: {
    name: "greet"
    description: "Greets someone."
    parameters: {type: "object" properties: {name: {type: "string"}} required: ["name"]}
  }
  use: "Greet"
}
```

`otter tool <file> ['<json arguments>']` (in `../otter`) prints a script's
definition or calls it, as the node would. A script that does not compile is
not offered, its diagnostics in `node.log`; a changed one is compiled again
at the next turn. A tool that only computes runs without asking; one that
reaches the network, files, the clock or waits asks as an MCP call does
(`run [otter:<skill>]: <tool> <arguments>`), unless allowed or yolo is on;
`always` grants it in the thread's place. Its files are confined to the
directory the thread works in.

```toml
[otter]
allow = ["google_flights_*"]        # run without asking
timeout_secs = 120                  # a call's longest run (at most 3600)

[otter.places."global"]
allow = ["greet"]                   # added by "always" answers (or by hand)
```

More: [setup §5a](setup.md#5a-tools-written-in-otter).

## Each thread's tools and skills

Every thread has its own tool and skill sets, and they follow it to your other
machines. A new thread starts from the defaults (ordinary built-in tool groups
and listed skills on, `self_improve` off, MCP tools as `config.toml` enables them,
then the place's own settings, below); changes go on top and apply from the
thread's next turn. Availability still depends on the node and credentials:

```sh
stitch thread tools last                  # list: on or off, and where each runs
stitch thread tools last +github -web     # an MCP server on, the web tools off
stitch thread tools last +github__get_*   # some of a server's tools
stitch thread skills last -release        # a skill off for this thread
stitch chat --tools -shell,+web "…"       # shell off, built-in web tools on
stitch chat --detach --tools +github "…"  # a detached one (the `spawn` tool takes `toolset` too)
```

In the TUI, `/tools` and `/skills` list them; Enter turns one on or off, Esc
closes (for a new thread the changes go with its first message). The built-in
groups are `shell`, `web`, `viewer` (`view_file`, `zoom_image`), `files`
(`write_file`, `edit_file`, `commit_push`), `weave` (`spawn`, `send`, `wait`,
`threads`, `cancel`, `models`, `schedule`, `wake_at`, `notify`,
`report_issue`), `memory` (`recall`), `images` (`generate_image`), `apps`
(`show_app`), `browser` (`browser_*`) and `self_improve` (off unless turned
on by name; [setup §6](setup.md#6-tool-and-skill-sets-per-thread)); an MCP server
named like one of them is `<name>__*`. A call to a tool that is off says so,
and how to turn it on.

A project's checkout or a folder can say what its threads get, in
`.stitch/` at its top: `settings.toml`, and skills of its own next to the home
workspace's, listed only for its threads:

```toml
# ~/Mail/.stitch/settings.toml
enable = ["mail"]           # tools: an MCP server this node configures, a group, a pattern
disable = ["web"]

[skills]
disable = ["release"]       # a home workspace skill not wanted here
```

```
~/Mail/.stitch/skills/mail/SKILL.md     # name and description in its header, as usual
```

The node that has the directory reads it at each turn and publishes what it
says as a record, so your other machines (the phone included) know it for
threads in that place without the directory; there a place skill is read
with the `read_skill` tool. Settings can only turn on what a node configures:
servers are still defined in `config.toml` alone. The exact syntax:
[setup §6–7](setup.md#6-tool-and-skill-sets-per-thread).

## Where threads belong

A thread belongs to the place it starts in. Every git checkout is a project,
known by the first commit of its history, so every clone of it, on any of your
machines, finds the same threads, with nothing to set up and nothing written
into the checkout. A fork is the same project as what it was forked from until
you run `stitch project split` in it; from then on it, and its clones, have
their own. A repository re-created from a new first commit at the same origin
is a new project too, until you run `stitch project rebind` in its checkout:
then the old project, with its threads, follows it there. Outside git, a
thread is tied to its folder, and the same path
under your home directory is the same folder on every machine (`~/notes` on
the Mac is `~/notes` on the Linux box). For folders at different paths, run
`stitch folder link nas:/mnt/notes` in one (or `/link` in the TUI);
`stitch folder unlink` keeps a folder to its machine. A folder you later
`git init` takes its threads into the new project. In your home directory, or
with `stitch chat --global` or `/new global` in the TUI, a thread belongs to no
folder. Shell sessions start in the thread's checkout or folder.

### Folder files

A folder outside git also syncs its **files** between your machines, once
you allow it: the first thread in a new folder asks (an approval, with what
it holds: "3,412 file(s), 210 MB"), or answer from a shell with
`stitch folder sync --allow` (`--stop` keeps its files on each node). Yolo
does not answer it. Hidden directories in your home (`~/.cargo`,
`~/.config`), temporary and system directories never sync unless allowed,
and do not ask.

- **When:** before each turn on the folder, when it ends (changes credited
  to its thread), after every sync, and on `stitch folder sync`.
- **What:** any file up to 25 MB, at most 2,000 files and 250 MB a folder
  (`[folders] max_file_mb`, `max_files`, `max_mb`), minus ignore rules
  (`node_modules`, `target`, `.venv`, editors' leftovers, …, and the
  folder's own `.stitch/ignore`, one pattern a line). A git checkout inside
  the folder is not carried.
- **Conflicts:** text edited on two nodes merges three ways when the edits
  do not overlap; otherwise the later edit wins and the other is kept as a
  conflict copy (`notes (mac, 2026-10-14).md`). A deletion against an edit
  keeps the edit.
- **History:** `stitch folder log [path]` lists changes (when, node,
  thread), `stitch folder diff [path] [--from] [--to]` shows one; the app's
  Files does the same and edits text files.

Snapshots are records: what synced stays in every node's history. Allow a
folder only when you mean its files to be on all your machines. Design
§2.16 has the details.

`stitch threads` lists this place's threads and global ones, the most recent
first, each with its place (`--all`: every place). The TUI reopens the thread
you last had open where you start it; Ctrl+N starts a new one there, and Tab in
the thread list switches between this place and all of them.

Projects' records live in the data directory (`projects/`), never in the
checkout, and every node holds and syncs all of them along with the home
workspace: a project's threads are on a machine before you clone it there, and
every node is a backup of the others.

## Threads that start threads

Work that takes long, should go on after the terminal closes, needs another
model's view, or needs another of your machines runs as a thread of its own:
the same agent toward a goal, with nobody watching. Any thread can be continued
later, from wherever it was, with what it already did: threads weave.

```sh
stitch chat --detach "update the dependencies and run the tests" --budget-usd 2 --minutes 30
stitch chat --detach --on mac --context --parent <thread> "implement what we discussed"
stitch send <thread> "and add a test for it" --from <thread>
stitch send <thread> "stop: the API changed" --interject   # into the turn running now
stitch threads                   # [running], [waiting], [done], [over-budget], ...
stitch attention                 # its approval questions
stitch answer <question> y       # or: n [why]
stitch cancel <thread>
```

- **The model does it with tools:** `spawn` (a goal, here or on another
  machine, a model that machine runs, its tools or none for a plain question,
  starting from no conversation, this one, or any thread's), `send` (continue
  any thread, on the machine that owns it; with `interject`, a message to a
  thread at work goes into its running turn between steps, instead of waiting
  for the turn to end, and that turn's reply comes back), `wait` (have the
  reports now), `threads` (those it started or sent to, with their state:
  working, blocked on an approval with the exact command, idle, ended),
  `cancel` (stop one now; its report says what it did) and `models` (what a
  machine runs). When a thread it started waits for your approval, the
  thread that started it is told exactly what it asks, so it can tell you;
  answering stays yours.
- **In the TUI:** `/spawn [@node[:model]] <goal>` starts one from the open
  thread's conversation; alone it asks for the machine (this one first), then
  that machine's model, then the goal. `/send` picks a thread, then you type
  the message.
- **Reports:** when a spawned or messaged thread's turn ends, its reply comes
  back to the thread that asked as a report, delivered on the machine that
  thread lives on: it wakes that thread's agent, unless it was waiting for it.
  In the TUI the report and the answer to it appear in the open thread as they
  land; reports for other threads are printed as notices.
- **Approvals** it needs become questions: answer them with `stitch answer`, or
  in the TUI with Ctrl+T, then `y`, or `n` and a reason. `stitch cancel` stops
  a thread, on whichever machine it runs. A node resumes the detached threads
  it started when it restarts. `/drop` clears the TUI's attachments.
- **Settings:** `[detached] max_steps` and `keep_shells`, and budgets, in
  [setup §9](setup.md#9-detached-threads-and-budgets).
- **Read-only threads** (`spawn`'s `read_only`, `--read-only`, or spawned by
  a read-only thread) only look. Their shells write only temporary
  directories (the sandbox enforces it: not the checkout, its worktrees,
  build caches or the node's data). In that sandbox anything runs but what
  acts beyond it (pushing, uploads, remote shells, git but its reading
  subcommands, `tmux`, `docker`, `systemctl` and `stitch` but their
  listings, cloud and deploy tools, …), which is refused, never asked; in an
  unsandboxed session (`outside`, an SSH host, a machine without a sandbox)
  only the built-in read-only commands run (your `[shell] allow` lists and
  "always" grants do not apply; add to `[shell] read_only_allow` what you
  vouch for as reading only). Agents such as `claude` and writing commands
  such as `git commit` are refused whatever a list says, and
  `write_file`/`edit_file` are refused. Only you switch a
  thread's mode, never an agent: `stitch thread <id> --writable` or
  `--read-only` (prints the mode), `/read-only on|off` in the TUI (whose
  status line says `read-only`), or the Read-only toggle in the app's chat
  menu (a badge under the title shows it). Read-only holds from the thread's
  next tool call (its shells here close); writable from its next turn, and
  the model is told. `threads` and `stitch.live` show each thread's mode.

## Schedules and wake-ups

Schedules wake Stitch without an open client. They are pinned to a node and
place, and can continue one conversation or start a fresh one at every firing.
They spend model budget even while you are away, so review the goal and limits.

```sh
stitch schedule add --name ci-review --every "0 9 * * 1-5" --tz Europe/Rome \
  --read-only --budget-usd 1 --daily-cap-usd 2 \
  "Review recent CI failures; change nothing. Notify me if action is needed."
stitch schedule add --name reminder --at "in 2h" --budget-usd 0.25 \
  "Notify me to check the deployment."
stitch schedule list
stitch schedule next
stitch schedule show ci-review
stitch schedule pause ci-review
stitch schedule resume ci-review
stitch schedule run ci-review            # fire now on its pinned node
stitch schedule rm ci-review             # end it
```

`--on <node>` pins to another machine, `--place` selects the place, and `--fresh`
starts a new thread per firing. `--no-catch-up` skips a missed firing rather
than running it once when the node returns. `--reports-to <thread>` sends
reports there; `--wake` also wakes that thread (otherwise reports are kept for
you). Per-firing dollar, token and time budgets, `--max-steps`, and a UTC daily
dollar cap bound the work. See `stitch schedule add --help` for all flags.

`--check '<Otter code>'` or `--check-file <place-relative.otr>` runs a check
before a firing at no model cost. An empty result (`none`, `false` or empty)
means no model wakes; a nonempty result accompanies the goal. The agent's
`schedule` tool can manage the same schedules and asks before creating one.
Its separate `wake_at` tool continues an existing thread later, by time or a
check; it is not a recurring schedule and uses that thread's remaining budget.

## Secrets, variables and SSH

Provider keys are secrets named for the provider. New secrets sync sealed to
trusted nodes unless marked `--device-only`; subscription sessions stay local.
Values live in credential storage, not plaintext conversation records. A secret
can apply globally or only to one place. Nonsecret variables sync as records.

```sh
stitch secret set deploy-token --here       # prompts; scoped to this place
stitch secret set local-token --device-only
stitch secret ls                            # metadata, not values
stitch var set REGION eu-west-1 --here
stitch var ls
```

Shells receive secrets only where an `[env]` mapping names them; ordinary
variables are supplied by name. Output is redacted before reaching the model.
Do not deliberately echo secrets, attach credential files, or rely on redaction
as a general data-loss prevention system. See [setup §3](setup.md#3-providers)
for storage, precedence, scope and environment mappings.

SSH hosts are synced configuration, not full Stitch nodes. Agents use their
shell tools' `host` parameter to run on one; delegation with `spawn` is preferred
for a machine that already runs Stitch.

```sh
stitch ssh host add buildbox --address build.example.org --user builder --approval ask
stitch ssh host ls
stitch ssh key                             # this device's public key
stitch ssh authorize buildbox              # authorize this device, via a node that gets in
stitch ssh authorizations
stitch ssh hostkey ls
```

Hosts support jump hosts, agent/file/device keys or a named password secret.
`--approval relaxed` asks for operations reaching beyond the host's working
area, `ask` asks for non-read-only commands, and `allow` trusts the host whole.
Remote sessions use tmux where available to survive a dropped connection
(`--no-tmux` disables it). Inspect host identity before trusting a connection;
`stitch ssh hostkey rm <host>` forgets its pinned keys, so the next connection
trusts what it finds. The exact flags are in `stitch ssh host add --help`;
the protocol and trust model are in design §2.20.

## Loose ends, deletion and notifications

`stitch loose-ends` judges conversations with the quick model and lists those
waiting on you. The node also judges hourly by default. Use `--list` to list
without a model call, or `--dismiss <thread>` to hide an item until that thread
moves again. The dashboard gives a live view of threads, approvals and jobs
without using a model.

Loose ends can also act for you, by your own rules (off by default):

```sh
stitch loose-ends --rules edit       # in your words: what to continue, what to dismiss
stitch loose-ends --auto dry-run     # record what it would do, do nothing
stitch loose-ends --auto on          # continue or dismiss what the rules cover
```

The judge proposes an action only where a rule clearly covers a conversation.
Before anything is sent, your default model checks the proposed message against
the rules. The message goes into the thread marked as sent by loose ends, not
typed by you, and the thread's normal approval rules still apply. Loose ends
never answer approvals or questions, never continue a conversation twice in a
row without a message from you in between, and continue at most 3 a day per
conversation. You get a push for each action. The web client's loose ends panel
has the same toggle (Off / Dry run / On), the rules box, and shows on each row
what was done or would be done. The setting and the rules sync to every node; a
node's `[loose_ends] auto` overrides it there.

`stitch thread rm <id>` deletes a thread and its compaction chain across nodes,
with a confirmation. Forks and spawned children stay; `--spawned-by <id>` adds
that thread's immediate children, not all descendants. `--yes` skips the CLI
confirmation: use it deliberately. Thread deletion is not a command for wiping
all copies of attachments or external provider data.

```sh
stitch notify "The build passed" --title "Build"
stitch apns status
stitch apns test
```

Notices and approval questions reach paired phones. Their content is sealed for
each phone before going through APNs or the app team's relay. A node can instead
use an APNs key for a build signed by its own team (`stitch apns set`), or a
custom relay (`stitch apns relay`). See [relay setup](../relay/README.md).

The native app runs chats, web reading, Otter tools and sync itself, but has no
local terminal or desktop Chrome profile. Delegate desktop work to another
node. iOS limits background execution: chats can suspend and resume; it is not
an always-on replacement for a desktop node. The app's guide is kept with the app
and design §5 for background behavior and cross-node continuation.

## Integrations and experimental features

- **Discord and model proxy:** `stitch discord` connects a bot to a node;
  `stitch proxy` keeps upstream API keys away from clients and applies token
  budgets and rate limits (`stitch discord --help`, `stitch proxy --help`:
  bot setup, allowlisted guilds, proxy tokens).
- **Harness issues and self-improvement:** agents can file Stitch bugs with
  `report_issue`; `stitch issue` manages them. Fixers and automated landing are
  opt-in, with separate budgets, protected paths, CI and user approvals.
  Read [self-improvement](self-improve.md) before enabling it.
- **Encrypted vault:** `stitch vault` is an experimental opt-in backup and
  store-and-forward protocol with inert recovery previews. It is separate
  from peer sync, has no hosted production service, and needs further security
  review. Start with [the protocol](vault-protocol.md).

For a custom Responses endpoint, use `kind = "responses"` and a `base_url`
ending at the version path (for example `https://api.openai.com/v1`). Existing
`kind = "openai"` configurations keep using Chat Completions for API keys.
