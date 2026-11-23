# Terminal UI reference

`stitch` with no command opens the TUI. It writes the conversation into the
terminal's own scrollback as plain text (select, copy, scroll and search with
the terminal); only the input, the streaming reply and the status line stay
live at the bottom. Behavior in depth: [features](features.md#the-root-agents-shell).
CLI commands: [cli.md](cli.md).

The list below mirrors `/help` (`HELP` in `crates/stitch-cli/src/tui/app.rs`);
keep the two in step.

## Slash commands

`/` alone opens a palette you filter by typing. Commands that need a choice
(`/model`, `/spawn`, `/send`, `/threads`, `/memory`, `/link`, `/login`…) offer a
list when given no argument.

| Command | Does |
|---|---|
| `/model [provider/model]` | Pick from the providers' models, or name one; `/model default` follows the synced default |
| `/spawn [@node[:model]] <goal>` | Start a thread toward a goal from this conversation, here or on another machine; alone: pick the machine, then its model |
| `/send [thread] [message]` | Continue any thread; its reply comes back here |
| `/threads` | Pick a thread (Tab: here and global, here only, all) |
| `/parent` | Open the thread that spawned this one, or that it reports to (Ctrl+P) |
| `/children` | The threads this one spawned or that report to it |
| `/new [global]` | A new thread here, or in no folder |
| `/rename <title>` | Name this thread |
| `/pin` | Pin this thread first in lists (again: unpin) |
| `/quote` | A new thread with this one attached as quoted material |
| `/search [--here] <words>` | Every thread's messages, by words and meaning; Enter opens one |
| `/fork [N] [-a] [-t] [-s] [@m12]` | A new thread continuing this one: all or the last N messages, `-a` no attachments, `-t` no tool calls, `-s` a summary of what is not kept, `@m12` up to that message; alone: a form |
| `/summarize` | A new thread continuing this one from a summary |
| `/compact [N]` | A new thread: a summary, then the last N messages (10) |
| `/attach <file>` | Attach a file to the next message |
| `/drop` | Drop the attached files |
| `/memory [on\|off]` | This thread in memory or out of it |
| `/read-only [on\|off]` | This thread read-only (shells write only temp) or writable |
| `/run-on <node\|here>` | Run this thread's turns on another node, streaming here; `here` brings it back |
| `/tools`, `/skills` | This thread's tools and skills; Enter toggles one |
| `/link [--force] node:path` | Make this folder the same as another node's |
| `/unlink` | Keep this folder to this node |
| `/jobs` | What runs in the background: shell jobs, detached threads (Ctrl+B; `x` stops one) |
| `/yolo [on\|off]` | Shell commands run without asking (the status line says so) |
| `/copy` | Copy the last reply (Ctrl+Y) |
| `/app [name]` | Open the latest app this thread showed, or a pinned one, in the browser |
| `/login [provider] [--file]` | Sign in to a subscription, or GitHub |
| `/logout [provider]` | Sign out |
| `/auth [provider]` | Subscription sign-in status |
| `/help` | The commands and keys, in the scrollback |
| `/quit` | Leave |

## Keys

| Key | Does |
|---|---|
| Enter | Send (while a reply runs: queue, given to it after its current tool calls) |
| Shift+Enter, Alt+Enter, Ctrl+J, `\` then Enter | New line (Shift+Enter where the terminal reports it: kitty, WezTerm, Ghostty, Rio, iTerm2 with CSI u) |
| Esc | Stop the reply (a running command gets Ctrl+C; queued messages return to the input) |
| ↑ (while queued) | Take the last queued message back to edit |
| Esc Esc (empty input) | Pick one of the thread's messages: `f` fork from here, `o` the fork form, `e` edit and resend, `c` copy |
| `@` | Pick a file of this folder; a text file up to 256 KB is attached, others named for the agent |
| Ctrl+V | Attach the clipboard's image (or a file copied in Finder); dropped files attach too |
| Ctrl+O | Threads |
| Ctrl+N | New thread |
| Ctrl+P | Parent thread |
| Ctrl+T | Attention: approvals and what waits on you (`y`, or `n` and a reason); also returns from the sign-in panel |
| Ctrl+B | Background jobs |
| Ctrl+Y | Copy the last reply |

The status line shows the model, `⚑ N` while N questions wait, `yolo`,
`read-only`, memory state, `↑`/`↓` for parent and children, and each turn's cost.
