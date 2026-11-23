# Stitch

**A local-first agent orchestrator for every machine you own.**

Stitch runs a small Rust node on each of your machines. The nodes pair with
each other and sync your conversations, agents and their work peer to peer,
with no Stitch server in the middle. You talk to it from a terminal UI, the
command line, a web client in your browser or the phone; it does the work
through real shells, tools and other agents, and brings back what needs you.

> **Status: early and moving fast.** Stitch is used every day by the people
> who build it, but formats, commands and defaults still change. Expect rough
> edges, and back up `~/.stitch` before upgrading across big changes.

## What it is

- **A node on each machine.** One process (`stitch`) holds your threads, runs
  the agents and their tools, and keeps everything as immutable records on
  disk: Markdown with TOML front matter and JSON Lines transcripts, indexed in
  SQLite. Nodes sync those records with each other over
  [iroh](https://iroh.computer); every node is a backup of the others.
- **Threads that weave.** A conversation is a thread. Any thread can spawn
  detached threads toward a goal, on this machine or another, on another
  model, with budgets in dollars, tokens or minutes; send to them, wait for
  their reports, continue any of them later. Schedules wake threads on
  their own, after a cheap check.
- **Agents with a real shell.** Persistent interactive sessions: programs
  that prompt, full-screen TUIs, background jobs and watches. Shells are
  sandboxed on Linux (Landlock) and macOS (Seatbelt), and what reaches
  beyond the sandbox becomes a question for you, answered from any of your
  devices.
- **A terminal UI and a web client.** `stitch` opens the TUI; `stitch client`
  opens the node's own web client in your browser (conversations, a home view
  of what needs you, approvals, a map of your threads). The CLI does the rest
  and is scriptable.
- **Tools.** Reading the web in headless Chromium, PDFs, spreadsheets and
  images; your own Chrome through an extension; MCP servers; SSH hosts;
  skills; small HTML/JS apps agents make for you, with state that syncs.
- **Tools written in [Otter](https://github.com/sinkingsugar/otter).** Otter
  is the language and runtime Stitch's tools and skills are written in:
  type-checked before they run, with their effects (network, files, time)
  known up front.
- **Many providers.** Anthropic, OpenAI and Grok (API keys or your
  subscription), Gemini, OpenRouter, Ollama, llama.cpp and other
  OpenAI-compatible endpoints.
- **Memory.** Optional on-device semantic memory (EmbeddingGemma 2 through
  llama.cpp, with [engram](https://github.com/sinkingsugar/engram)) that
  recalls what earlier conversations learned.
- **Projects found by themselves.** Every git checkout is a project, known by
  its first commit, so its threads follow it to every clone on every machine.

Official iOS and visionOS apps run a full node on the device (chats,
pairing, sync, approvals); their push notifications go through Fragcolor's
push relay, which cannot read their content. The apps are not part of this
repository.

## Platforms

| Platform | |
|---|---|
| Linux | Node, CLI, TUI, web client; shells sandboxed with Landlock |
| macOS | Node, CLI, TUI, web client; shells sandboxed with Seatbelt |
| Windows | Through WSL2, with the Linux build; there is no native Windows build |
| iOS, visionOS | The official apps (a node in process, no local shells) |

## Install from source

You need Rust (the version is pinned by `rust-toolchain.toml`; rustup picks
it up), CMake (Ninja is used when present), a C/C++ toolchain and clang (for
bindgen), and git. Otter and engram are git dependencies Cargo fetches at the
commits `Cargo.lock` pins.

```sh
git clone https://github.com/sinkingsugar/stitch
cd stitch
git submodule update --init          # llama.cpp
cargo install --locked --path crates/stitch-cli
```

That installs two binaries into `~/.cargo/bin`: `stitch` (the node, CLI and
TUI) and `stitch-shell-helper`. Keep them together: on macOS the node starts
shells through the helper. Instead of `cargo install` you can run the
install script, `deploy/node/install.sh` (also `just install`, with
[just](https://github.com/casey/just)): the same release build, put in place
atomically, and on macOS signed with your Apple Development identity so the
keychain does not ask again after each rebuild.

Running the node as a service, updates and rollbacks:
[setup §1](docs/setup.md#1-install-update-restart).

## Quick start

```sh
stitch setup --name laptop          # name this node and create its home workspace
stitch auth login anthropic         # or openai / grok; or: stitch key set anthropic
stitch                              # the TUI
```

From the shell:

```sh
stitch chat "summarize the TODOs in this repo"
stitch chat -t last "now fix the first one"
stitch chat --detach --budget-usd 2 "update the dependencies and run the tests"
stitch threads                      # what runs, waits or is done
stitch attention                    # what waits on you
stitch answer <question> y
stitch client                       # the web client in your browser
```

Commands that need the node start it in the background. It keeps running,
and restarts by itself, once idle, into a newer build you install.

Pair another machine:

```sh
stitch invite                       # on a node you have: a QR code and a ticket
stitch setup --name desktop --no-home   # on the new machine
stitch join <ticket>
```

In the TUI, `/` lists the commands, `/help` prints them with their keys,
`/model` picks a model and `/spawn` starts a thread here or on another
machine.

## Data and trust

Local-first is not offline-only: the model providers, web tools and search
services you use receive your requests, and iroh may use relays to connect
your nodes (`stitch setup --local` keeps to direct addresses). Pair only
devices you trust with your records. Secrets sync sealed unless marked for
one device only; subscription tokens stay on the machine that signed in.

Approvals and sandboxes reduce risk; they do not make generated code safe.
Use read-only threads and budgets for unattended work. `stitch yolo on` lets
shell commands run without asking. The end-to-end encrypted vault is
experimental and not a backup service yet.

## Documentation

The [documentation index](docs/README.md) lists everything. To start:

- [Setup](docs/setup.md): install, services and updates, pairing,
  providers, approvals and the sandbox, MCP, places and skills, memory,
  troubleshooting.
- [Features](docs/features.md): a tour of what Stitch does.
- [Commands](docs/commands.md) by task, and the full [CLI reference](docs/cli.md).
- [Terminal UI](docs/tui.md): slash commands and keys.
- [Design](docs/design.md): the architecture and the decisions behind it.

## Repository layout

| Crate | What it is |
|---|---|
| `stitch-records` | Record IDs, references, the record and transcript formats |
| `stitch-store` | Record roots, the blob store, the SQLite index, record sync |
| `stitch-net` | The node-to-node protocol over iroh: sync, blobs, pairing, trust |
| `stitch-node` | The node: config, credentials, the agent loop and tools, the client protocol |
| `stitch-providers` | Model providers over HTTP and server-sent events |
| `stitch-shell` | Interactive shell sessions on PTYs, and the sandbox |
| `stitch-ssh` | SSH connections and remote sessions |
| `stitch-memory` | Memory: engram over EmbeddingGemma 2 |
| `stitch-llama` | llama.cpp in process, for embeddings |
| `stitch-cli` | The `stitch` binary (node, CLI, TUI) and `stitch-shell-helper` |
| `stitch-ffi` | The node for Apple clients, through UniFFI |
| `stitch-client` | Publish events to a node and ask its threads, from Rust |
| `stitch-proxy` | A key-holding proxy for model APIs, with per-token budgets |
| `stitch-discord` | A Discord bot in front of a node |
| `stitch-vault` | The experimental end-to-end encrypted vault's protocol and client |

Also: `apps/client` (the web client), `apps/chrome` (the browser
extension), `relay/` (the push relay), `deploy/node` (the service units and
the self-update), `examples/`, `tools/`.

## Contributing

Issues and pull requests are welcome: read [CONTRIBUTING.md](CONTRIBUTING.md)
first. Contributions are accepted under a Contributor License Agreement.

```sh
just quick          # while developing: fmt, clippy and tests of what changed
just check          # the full set, as CI runs it
```

## License

Stitch is under the **Functional Source License, Version 1.1, Apache 2.0
Future License** (FSL-1.1-ALv2), © Fragcolor Pte. Ltd.: see
[LICENSE](LICENSE). In plain words:

- You may use, copy, modify and redistribute Stitch for **any purpose except
  a competing use**: offering it, or something substantially like it, to
  others as a commercial product or service. Using it at work, for your own
  business, in research or education, or to provide services to a client is
  fine.
- **Each release becomes Apache-2.0 two years after it is published**, with
  no restriction left.

Otter and engram are separate projects under MIT OR Apache-2.0. Third-party
components and their licenses are listed in [notices/](notices/README.md).
