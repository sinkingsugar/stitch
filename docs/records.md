# Stitch: records and the store layout

Status: decided 2026-10-11, not implemented. Each section states the format, with samples, and ends with the options that were rejected and why.

This is the format for milestone 1 (design §5). It follows the rules of design §2.5:
- **immutable files** with unique names, created exclusively;
- **derived state:** status is computed from references; a genuinely mutable value is a small timestamped record, latest wins;
- **transcripts** in compacted segments, never per token;
- **blobs** by content hash, beside git, not in it;
- **indexes** derived, local and gitignored.

The prototype is Otter's `.agent-handoffs/` (`skills/review-handoff/references/records.md` there): Markdown records named `<date>-<sha>-<author>-<suffix>.md`, a filename stem as the record ID, `<id>#F1` for findings, plain bullets for fields, no parser. What works there and carries over: immutability, exclusive creation, stable finding IDs, status from references, and records people and agents read directly. What does not scale: fields as free-form bullets (a node must parse them), relative links between records (they break across repositories), and no notion of threads, attachments or mutable values.

## 1. Encoding

Every record except a transcript segment is **Markdown with TOML front matter**: strict, typed fields the node depends on, and a Markdown body that people and agents write and read. A question a worker asks the user:

```markdown
+++
v = 1
id = "2026-10-11T143207Z-mbp-k3f9"
kind = "question"
created = 2026-10-11T14:32:07.412Z
author = { node = "mbp", run = "2026-10-11T140112Z-mbp-q7cd" }
choices = ["Keep TOML", "Switch to YAML"]

[refs]
task = "2026-10-11T135840Z-phone-a2x8"
+++

# Front matter: TOML or YAML?

The record parser is half written. TOML needs no implicit typing rules and
`serde` reads it directly; YAML is what most Markdown tools expect.
Which one should records use?
```

Rules:
- the file starts with `+++\n`; front matter ends at the next line that is exactly `+++`;
- `v` (format version), `id`, `kind`, `created`, `author` are required on every record;
- the body's first `# ` heading is the record's title, if it has one; a title is never stored twice;
- unknown front matter keys are kept and ignored, so newer nodes can add fields without breaking older ones;
- UTF-8, `\n` line endings, final newline.

Rejected:
- **YAML front matter:** implicit typing (`no` is false, `1.10` is a float) and larger parsers, for the one advantage that editors hide it.
- **JSON:** the body becomes an escaped string, poor for long text people read.
- **Markdown bullets,** as `.agent-handoffs` today: the best to read, but fields need a grammar of their own and parse fragilely.

## 2. Identifiers and file names

A record's ID is its file name without the extension. It has to be unique across every node and repository of the user, created without coordination, and sort by time. It must not contain anything mutable (no title slug: titles change by a `set` record, §6).

The ID is **time, node, random**: `2026-10-11T143207Z-mbp-k3f9`.
- UTC time to the second, the node's short name, four random base32 characters;
- the node name makes IDs from different nodes distinct; exclusive creation plus a retry with a new suffix makes them distinct on one node;
- readable: when and where it was written is in the name.


Node names are short, chosen at setup (`phone`, `mbp`, `nas`), unique within the user's nodes (the home workspace lists them, §7), and `[a-z0-9]{1,12}`. Random suffixes use the lowercase base32 alphabet without `l`, `o`, `0`, `1`. Every file the node writes, transcript segments included, is created exclusively and retried with a new suffix on a collision.

**Sub-items** keep the handoff convention: `<id>#F1` names finding F1 inside a review, `<id>#Q2` a question inside a report. Sub-item IDs never change once published.

**References across repositories.** IDs are unique on their own, so a reference is just the ID. The node's index finds it in whatever repository holds it. When the target is in another repository that a node may not hold, the reference carries a hint: `"stitch-home:2026-10-11T135840Z-phone-a2x8"`, where `stitch-home` is the target repository's ID (§7). A reference that does not resolve is shown as unresolved, never an error.

Rejected:
- **ULID or UUIDv7:** standard and sortable, but opaque; a listing tells people nothing.
- **Content hash:** needs a canonical form, and the ID is unknown until the record is final, which makes drafts and self-references awkward.

## 3. Repository layout

Records are **flat by month**, with the kind in the front matter; transcripts get one directory per thread.

```
.stitch/
  repo.toml
  records/
    2026-10/
      2026-10-11T135840Z-phone-a2x8.md      kind = "task"
      2026-10-11T140112Z-mbp-q7cd.md        kind = "run"
      2026-10-11T143207Z-mbp-k3f9.md        kind = "question"
  threads/
    2026-10-11T140112Z-mbp-q7cd/            one directory per thread, named by its ID
      2026-10-11T141530Z-mbp-77ha.jsonl     transcript segments
      2026-10-11T143207Z-mbp-m2pz.jsonl
```

- one rule for where any record lives (the month comes from its ID), so writing needs no knowledge of kinds;
- new kinds need no new directories;
- a month directory stays a reasonable size: a busy month is a few thousand files.

Transcript segments are numerous and only make sense with their thread, hence their own directories.

Rejected:
- **By kind** (`tasks/`, `questions/`, ...): every new kind is a layout change, and a record whose meaning shifts (a question that becomes an approval) sits in the wrong place.
- **By task** (`tasks/<id>/` holding everything under it): a record relating two tasks (a handoff) belongs to only one, and records without a task need another place anyway.

**The record root** is the directory holding `repo.toml`, `records/` and `threads/`. Where it is (design §2.5), on every node, phones included:
- the home workspace: `home/` in the data directory (or where `stitch setup --home` put it). It also has `agents/` and `tools/` for Otter scripts: ordinary mutable files, not records, since they are code;
- a project: `projects/<repository ID>/` in the data directory; never in the project's checkout;
- a folder place's synced files (design §2.16): `folders/<repository ID>/records/` in the data directory, the ID `f-` and 16 hex digits of the BLAKE3 hash of the folder's identity (`~/notes`), so every node finds the same one. Beside it the node keeps `state.json` (the snapshot its files are at, and what it knew of each file at the last scan; never synced) and, on a phone, `files/`, its copy of the folder.

The tree above shows a record root as `.stitch/`, its name in the first drafts.

Record paths are relative to the record root (`records/2026-10/<id>.md`), and that relative path is what sync exchanges (§7).

`repo.toml`:

```toml
v = 1
id = "stitch-home"           # stable repository ID, used in cross-repository hints
role = "home"                # or "project", "folder"
created = 2026-10-11T13:20:00Z
```

## 4. Kinds

The kinds milestone 1 needs. Relations (`[refs]` keys) are listed per kind; status is derived from them in §8.

| Kind | Written by | Purpose | Main refs |
|---|---|---|---|
| `thread` | user, node | A conversation; detached (`detached = true`, a `goal`, a `budget`, `tools = false` for a question to a model, `approval = "read-only"`: what the rules do not allow is refused, never asked, and so for what it spawns) when it runs on its own; spawned or forked, it starts with the conversation it `continues` (to `continues_call` or `continues_at`; a fork also `continues_msg` up to a message, `continues_attachments = false`, `continues_tools = false`, `continues_last = N`, and a `summary` standing for what it leaves out; `compacted = true`: the thread it continues went on in it, with `compacted_model`, `compacted_window` and `compacted_tokens`, the model whose context window it outgrew, that window and what its last step sent, for the divider clients show, design §2.15). Its transcript is in `threads/<id>/` | `spawned-by`, `reports-to`, `scheduled-by`, `forked-from`, `assignment`, `continues` |
| `schedule` | user, thread | When to wake a detached thread (`at`, or `every` as a five-field cron expression in `tz`), with a `name`, `goal`, `place`, `model`, `max_steps`, `read_only`, `budget` per firing, `daily_cap`, `thread` (`continue`: one thread resumed at each firing, the default; `fresh`), an Otter `check` (or `check_file`) run first at no model cost, `catch_up`, `wake`, `self_wake = true` and `until` (a thread's wake-up and its deadline), the `place` key (and `folder`, as a thread's), and the `node` it is pinned to. Changes are `set`s: `state` (`active`, `paused`, `ended`) and its fields (design §2.3). Its threads point to it (`scheduled-by`); when it last fired is the pinned node's own state, not a record | `reports-to`, `thread` (a wake-up's thread) |
| `task` | user, worker | A goal with budget and permissions. Superseded by detached threads (design §2.3); kept until they replace its status rules | `parent`, `spawned-by` |
| `run` | node | One worker executing toward a task on one node. Superseded by detached threads, as `task` | `task`, `assignment` |
| `report` | worker | A checkpoint or final report; findings as `## F1 —` sections. A detached thread's report names `thread` and optionally `reports-to`; cancellation before its start is confirmed also names its spawn `assignment`, preventing a late claim even when no thread record exists yet; so does a `failed` one (`error`: why) from the node that gave up starting it (design §2.7) | `task`, `run`, `thread`, `reports-to`, `assignment` |
| `review` | worker | Findings about code or another report, as in the handoffs | `task`, `reviews` |
| `resolution` | worker | Responses to findings | `addresses` |
| `verification` | worker | Independent check of resolutions | `verifies` |
| `question` | worker | Blocks on the user; `choices` makes it an approval | `task`, `run` |
| `answer` | user, node | Answers a question; a node writes one to withdraw an approval nothing asks any more (its loop stopped, its thread reported, the caller of a call for another node is gone: declined, `withdrawn = true`; design §2.3, §2.14), or to let one go when yolo is turned on while it waits (allowed) | `answers` |
| `handoff` | worker | Passes work to another task or worker | `from`, `to` |
| `assignment` | user, node | Work for another node (design §2.7): `node`, `goal` (the message), `thread` (the new thread's ID, or with `continue = true` the thread to continue), optional `model` (for `quick` or `default`: the model the writing node resolves it to, with the name as `model_as`, which the taking node resolves itself; a node older than those names reads only `model`), `budget`, `tools = false`, `memory = true` (the new thread stays in memory; without it, it starts out of memory), `recall = true` (out of memory, it may still search memory; without either, it recalls nothing, as an assignment from before meant), `approval = "read-only"`; with `continue = true`, `interject = true`: woven into the turn running on the thread, else a turn of its own; `chat = true`: the user typed it into the thread on the writing node, which follows the turn live (design §2.7, "Continue on another node"), and the turn's reply is kept for the user rather than waking anything; such a message carries its files as the record's attachments, and `model`, the model its client picked, used when the taking node runs it; where the conversation was, `continues_call` (the spawning tool call) or `continues_at` (a segment). A new thread is claimed by starting it, a message by a `claim`. A message (`continue = true`) also references its thread: the thread reads as working from it until a report after it (design §2.7) | `reports-to`, `continues`, `thread` |
| `claim` | node | Takes an assignment: a message for a thread (no lease), or later one with a lease. With `thread` instead: a turn of that detached thread, which had reported, starts on this node (a turn nobody typed); the thread reads as working until its own report after it, and the node resumes the turn at its next start if it stopped before that report (`wake = false`: its report is kept) (design §2.7) | `claims`, `renews`, `thread`, `reports-to` |
| `close` | user, worker | Ends a task: `result = "done" \| "dropped" \| "failed"` | `closes` |
| `set` | user, node | A mutable value (§6) | `target`, `supersedes` |
| `capabilities` | node | A node's capabilities (design §2.7); a newer one supersedes. Now: the MCP servers it configures and the tools of those it shares (design §2.14) | `supersedes` |
| `toolset` | user, node | Changes to a thread's tool and skill sets (design §2.14), folded in order | `thread` |
| `place` | node | A place's own settings and skills as one node read them from `<place>/.stitch/` (design §2.14), in the home workspace; an empty one when that node has the place without `.stitch/` (deleted) | |
| `call` | thread | A call of a tool shared by another node, queued while that node was offline (design §2.14); taken by a `claim`, answered by a `report` | `thread`, `reports-to` |
| `peer` | node | Admits one of the user's nodes (design §2.6), in the home workspace | |
| `snapshot` | node | A change of a folder place's files (design §2.16), in the folder's store; it attaches the blobs it adds (trees and files) | |
| `device` | node | A phone's push token (design §5, milestone 4), in the home workspace; the latest of a node stands | |
| `app` | user | A pinned app (design §2.10), in the home workspace: a version of it, its name and icon; pinned again, a newer one supersedes the heads of the same app; one with `removed = true` unpins it | `supersedes` |
| `revoke` | user | Withdraws a `peer` record | `revokes` |
| `secret` | node | A secret synced sealed (design §2.19), in the home workspace: `name`, `scope`, `set_at` (when its value was set: the latest stands), `sealed` (a table, node → its ciphertext, base64), `rotate` (removed nodes that held it), or `removed = true` | |
| `seal-key` | node | A node's X25519 sealing key, public half (`node`, `key`, base64), what secrets are sealed for it with; the latest of a node stands | |
| `rewind` | user | A thread goes on from one of its messages (design §2.15, "Rewinding a thread"): `segment` and `upto` (the message's ID; neither: from its start). The latest rewind's branch is the thread's main line (§5.1); one not written by the user counts for nothing | `thread` |
| `deletion` | user | Deletes threads for good (design §2.5, "Deleting a thread"): a tombstone, in the threads' repository, kept forever. Every node purges what it names and what goes with it, and refuses them in sync (§7); one not written by the user counts for nothing | `deletes` |
| `issue` | user, node | A harness issue: a bug in Stitch itself (docs/self-improve.md §2.2), in the home workspace: `title`, `area`, `build`, `version`, `place`, the report as its body (what happened, expected, evidence). Its state is `set`s: `state` (`open`, `fixing`, `fixed`, `dropped`; none is open), `fixer` (a thread), `fix` (a commit), `note` | `thread` (the thread that filed it) |
| `variable` | user | A variable (design §2.19), in the home workspace: `name`, `scope` (`global`, `project:<repo>`, `folder:<key>`), `value`, or `removed = true`; the latest of a name and scope stands | |

Fields the index reads to derive status (§8); everything else in a record is free:

| Kind | Field | Values |
|---|---|---|
| `task` | `budget` | `{ usd = 5.0, tokens = 2000000 }`, either or both |
| `project` | `repo`, `root`, `name`, `remote`, `split` | the project's record store, the first commit of its history, a name, its normalized `origin` (`github.com/org/repo`), and `true` for a fork split off (design §2.3); in the home workspace |
| `memory` | `model`, `facts` | the encoder's tag (hex) and the facts in order (design §2.9): `{ at = "repo:thread#m3", embedding = "<base64 int8>", demand = 0.1, own = true, supersedes = true }` an observation (`own`: a Strengthen into a sibling of its thread keeps its own memory; `supersedes`: a Strengthen of it is the newer version), `{ shown = ["<handle hex>"], credit = ["<handle hex>"] }` what an exchange showed and what it used, `{ penalize = [...] }` what it dismissed, `{ sleep = true }` a sleep, `{ gist = "…", embedding = "…", kept = [...], dropped = [...] }` a gist, `{ thread = "<id>", remember = false }` a thread out of memory (or back; a spawned thread's is written before its first step unless it was spawned with `remember`), with `recall = true` when it may still search memory itself (a spawned thread's default; read by a build from before as out of memory altogether) |
| `thread` | `folder` | `{ node = "mbp", path = "~/notes" }`: the folder a home-workspace thread belongs to, home-relative where it is in the home directory (design §2.3); none on a global thread or one in a project repository |
| `thread` | `system_prompt` | the blob hash of its system prompt, one of its `attachments` (`system-prompt.md`), shared by threads with the same prompt; a thread written before 2026-10-13 has the text as `system` |
| `folder` | `link`, `unlink`, `location` with `project`, `sync` with `allow` | `link = ["mbp:/srv/data", "nas:/mnt/data"]` joins two folders; `unlink = "nas:~/notes"` keeps that one to its node and ends the links made before it; `location = "mbp:~/tool"` with `project = "p-…"` folds that folder into a project; `sync = "~/notes"` with `allow = true` lets that folder's files sync (`false`: kept on each node; design §2.16). Read in the order written; in the home workspace |
| `report` | `final`, `outcome`, `news` | `true` ends its run; how a thread's turn ended: `done`, `blocked`, `not-done`, `failed`, `stopped`, `over-budget`, `cancelled` (design §2.7); `news = false`: the turn had nothing new to say, and the report is not delivered; `wake = false`: kept for the user, waking nothing (also on a `thread` spawned so, and on an `assignment` sent so); `woven = true`: the reply of a turn to a thread whose message was woven into it (`send` with `interject`), not a report of the thread's own (its state and whether it reported ignore it) |
| `resolution` | `disposition` | `fixed`, `disputed`, `deferred`, `duplicate`, for every target in `addresses` |
| `verification` | `result` | `verified`, `still-failing`, `inconclusive`, for every target in `verifies`; mixed results are separate records |
| `question` | `choices`, `approval`, `command`, `session`, `grant`, `place`, `grant_browser` | a list; makes it an approval (`approval = true`, the `command` and the shell `session`, `mcp:<server>` or `browser` it is for); `always` among the choices when `grant` lists what that answer allows from then on (command prefixes; sites with `grant_browser = true`) in `place` (its key: `project:<repo>`, `folder:<key>`, `global`), saved by the node that asked. A browser question may carry a screenshot of the tab as an attachment (`tab.png`) |
| `answer` | `choice`, `withdrawn` | one of the question's `choices` (`always`: allowed, and its `grant` from then on); `withdrawn = true`: the node withdrew it (declined), not a decision any loop takes up |
| `claim` | `lease_until` | a date-time |
| `close` | `result` | `done`, `dropped`, `failed` |
| `set` | `field`, `value` | the field's name and its new value |
| `peer` | `name`, `endpoint` | the node's name and its endpoint ID (hex) |
| `toolset` | `tools`, `skills` | entries, each `+x` (or `x`) or `-x`: groups, MCP servers or patterns, skills (design §2.14) |
| `place` | `place`, `node`, `enable`, `disable`, `memory`, `skills_disable`, `skills` | the place's key (`folder:~/Mail`, `project:<repo>`), the node that read it, its settings (`memory` only when set), and `[[skills]]` with `name`, `description`, `text` |
| `capabilities` | `node`, `servers`, `shared` | the servers it configures (names), and `[[shared]]` with `server` and `tools` (`name`, `description`, `schema` as JSON text) |
| `call` | `node`, `server`, `tool`, `input` | the node to run it, the server, the tool (unprefixed), the arguments as JSON text |
| `app` | `app`, `name`, `icon`, `manifest`, `thread`, `removed` | the app's ID (its first record's; none on the first), its name, an SF Symbol's name, the version (the blob of its manifest, also attached), the thread it was pinned from, `true` when unpinned. A manifest is a JSON blob, `{"app": 1, "name": "…", "tree": "blake3:…", "entry": "index.html", "networking": false, "thread": "<id>"}`, its tree a folder's (`snapshot`); an app's state is `set` records on its thread, field `app:<key>`, the value JSON as text (empty: removed) |
| `device` | `token`, `env`, `key` | the APNs device token (hex); `sandbox` (a development build) or `production`; the public half of the phone's X25519 push key (base64), what pushes are sealed for |
| `snapshot` | `folder`, `tree`, `prev`, `changed`, `thread` | the folder's identity; the blob of its root tree; the snapshots it follows (none for a first, several for a merge); the paths it changed from the first of them (at most 200 listed); the thread whose turn made it. A tree is a JSON blob, `{"entries": {"<name>": {"t": "file", "hash": "blake3:…", "size": 12} \| {"t": "dir", "tree": "blake3:…"}}}`, names sorted |

Every record may also carry `attachments` (§5.3) and any of these refs: `relates`, `depends-on`, `supersedes`. A ref's value is one ID or a list of IDs; sub-items (`<id>#F1`, `<thread>#m12`) are valid targets.

## 5. Transcripts

A thread's messages, model replies, tool calls and results. Written as segments at checkpoints, never per token (tokens go over the live channel).

### 5.1 Segment format

Segments are **Stitch's own JSON Lines**: one event per line, the first line the segment header.

```jsonl
{"v":1,"segment":"2026-10-11T141530Z-mbp-77ha","thread":"2026-10-11T140112Z-mbp-q7cd","created":"2026-10-11T14:15:30.020Z","node":"mbp","prev":null}
{"t":"msg","id":"m1","at":"2026-10-11T14:01:12.300Z","role":"user","from":"user@phone","parts":[{"text":"Draft records.md for the store. Use the handoff format as the starting point."}]}
{"t":"msg","id":"m2","at":"2026-10-11T14:01:20.910Z","role":"assistant","model":"claude-opus-5-5","parts":[{"text":"Reading the handoff format first."}]}
{"t":"tool_call","id":"c1","at":"2026-10-11T14:01:21.004Z","name":"Read","input":{"file_path":"/home/me/src/otter/skills/review-handoff/references/records.md"}}
{"t":"tool_result","id":"c1","at":"2026-10-11T14:01:21.090Z","ok":true,"output":{"text":"# Record format\n\nStore UTF-8 Markdown under ..."}}
{"t":"msg","id":"m3","at":"2026-10-11T14:15:29.700Z","role":"assistant","model":"claude-opus-5-5","parts":[{"text":"First draft is in docs/records.md."}]}
{"t":"usage","at":"2026-10-11T14:15:29.700Z","model":"claude-opus-5-5","input_tokens":48211,"output_tokens":6120,"cache_read_tokens":40102,"usd":0.41}
```

- `rewind` and `upto` in a header: the segment starts the branch of that `rewind` record, following message `upto` of `prev` (what `prev` holds after that message and its tool calls is not on the branch). The main line is the latest rewind's branch (the longest chain within it), or while nothing continues that rewind, the line up to its message; with no rewind, as below;
- `prev` is the previous segment's ID, so a thread is a chain; two segments with the same `prev` are a fork (two nodes continued the same thread), shown as one, not merged silently: the thread is its main line, its longest chain (of those as long, the head written first, by `created`), the same on every node holding the same segments; what the other continuations said is told to the model with its context (design §2.4);
- event IDs (`m1`, `c1`) are local to the thread; a record cites a message as `<thread-id>#m3`;
- large tool outputs (over 64 KB) go to the blob store and the event holds `{"blob": "blake3:…", "bytes": …}`;
- `usage` events are what budgets are computed from (§8).
- a user `msg` may carry a `{"recalled": "…", "handles": ["…"], "measures": {…}}` part: memories recalled for it unasked (design §2.9), sent to the model as text, shown in a transcript as a marker; `measures` (optional) keeps how the note was chosen: `cue` (`distinctive`, `chars`, `novelty`, `ceiling`, `wandering`, `centered_max`, `fresh`) and `memories` (`handle`, `raw` and `centered` distance), for recalibrating; a memory record's `credit` and `penalize` facts name the note they judge in `note` (its message's place);
- a user `msg` may start with a `{"context": "…"}` part: the thread's instructions and skills (design §2.3) as they became when they changed since its system prompt was written; sent to the model as text, shown in a transcript as a marker;
- an assistant `msg` may carry `native`: `{"provider", "model", "content"}`, the provider's own content for that turn (for Anthropic, its content blocks, thinking blocks and signatures included). When the thread continues on the same provider and model, that content is sent back unchanged, as the provider requires; any other model gets `parts`. Anthropic's content is written packed (2026-10-13): a text block that is the message's own text is `{"type": "text", "at": [from, to]}`, a byte range of its `parts` text, and a `tool_use` block that is one of its step's `tool_call` events is `{"type": "tool_use", "call": "<id>"}`; a reader makes them whole before sending. Thinking blocks and anything else stay as they came.

For external workers the node also keeps their native stream (Claude Code's `stream-json`, Codex's `--json` events) as a blob named in the segment header, `"raw": {"blob": "blake3:…", "format": "claude-stream-json"}`, so nothing is lost and the translation can be rerun.

Rejected:
- **The native format only:** two vendor formats that change without notice, and the native loop would need a third; views, search and continuation would each handle all of them.
- **Markdown transcripts:** lossy for tool inputs and outputs, and fragile to parse back for continuation.

### 5.2 When a segment is written

- **a turn:** its steps are appended to the turn's open segment, `journal/<thread>/<segment>.jsonl` in the record root, as each is done (its tool calls run); read on the node with the sealed ones, never synced;
- **sealed** into `threads/<thread>/<segment>.jsonl`, the same name and content, when the turn ends, before the repository syncs (a turn then goes on in a new segment), and when the node starts or stops with one open; a last line cut short by a crash is left out.

A thread is a few files per turn at most (one, unless a sync came mid-turn). Segments written one per step before 2026-10-13 were merged by turn once, by every node the same way (design §2.5). `replaces` in a header is not used.

### 5.3 Attachments

An attachment is a reference, the same shape in a transcript part or in a record's front matter:

```json
{"att": {"hash": "blake3:9f2c4e…", "type": "image/heic", "name": "IMG_4121.HEIC", "bytes": 2481133}}
```

```toml
[[attachments]]
hash = "blake3:5a01d7…"
type = "application/pdf"
name = "lease-agreement.pdf"
bytes = 812044
```

A room scan is several attachments in one message: the rendered floor plan (`image/png`), the measured summary (`text/markdown`), the structured scan (`application/vnd.stitch.roomscan+json`) and the model (`model/vnd.usdz+zip`).

## 6. Mutable values

Titles, a task's priority, a thread's archived flag, when the user last opened a thread (`opened`, the user's), what loose ends judged of it (`loose-end`, a table naming the step judged) and a dismissal of that (`loose-end-dismissed`, the user's; design §2.10), a thread's mode (`approval`: `read-only` or `default`; only the user's `set` counts, design §2.2), where a thread runs (`runs-on`: a node's name, or `""` for back to where it is typed; only the user's `set` counts, design §2.7), node settings. A `set` record names a target and a field:

```markdown
+++
v = 1
id = "2026-10-11T151002Z-phone-r8vw"
kind = "set"
created = 2026-10-11T15:10:02.118Z
author = { node = "phone", user = true }
field = "title"
value = "Records format"

[refs]
target = "2026-10-11T135840Z-phone-a2x8"
supersedes = "2026-10-11T135840Z-phone-a2x8"   # the value it replaces: the creation record, or the previous set
+++
```

The current value follows the **supersedes chain, with time as tie-break.** Each `set` names the value it saw. Normally that makes a chain and its end wins. When two `set` records supersede the same value (two nodes edited offline), the later `created` wins and the conflict is kept visible in the index, so a view can show "changed on two devices". The heads are the `set` records nothing supersedes; an edit made while a conflict shows names every head it replaces (`supersedes` takes a list, like a merge commit's parents), which ends the conflict.

A `set` lives in the repository of its target.

Rejected:
- **Latest `created` wins:** a wrong clock on one node wins silently.
- **Hybrid logical clocks:** correct ordering without trusting clocks, one more field in every file, and still no answer for real concurrent edits.

## 7. Blobs, sync and nodes

**Blob store**, outside git, per node: `~/.stitch/blobs/blake3/9f/2c/9f2c4e…` (full hash as file name, two levels of fan-out).
- **Hash:** blake3: fast, Rust-native, tree-hashed for large files. The algorithm is part of every reference (`blake3:…`), so another one later is not a format break.
- **Sync** (design §2.11): phones upload what they create and fetch on demand; always-on nodes hold everything.
- **Rejected:** sha256 (slower; its one advantage, matching other tools, does not matter inside the store), and git LFS (every node fetches through an LFS server, which a set of the user's own machines over SSH does not have).

**Record sync** (design §2.6) reconciles, per repository ID, the set of record files two nodes hold:
- an **item** is a file of the record root: its relative path and the blake3 hash of its content. Its key is the path; paths sort by time within a directory, because IDs do;
- the **fingerprint** of a range of keys is the XOR of `blake3(path, 0x00, content hash)` over its items, with the item count;
- one side sends the fingerprint of the whole key range. The other compares with its own: equal ranges are done; a range with few items (proposal: 32 or fewer) is answered with its list of items; a larger one is split into 16 subranges by key, each answered with its fingerprint. Both sides then send the files the other lacks;
- in sync, two nodes exchange one fingerprint. Two nodes differing by a day of records exchange a few rounds and those files;
- the same path with different content is never expected (files are immutable); it is reported as corruption, and neither side overwrites;
- reconciliation is a message exchange independent of the transport.

Over the network (design §2.6), a sync is one QUIC stream on the ALPN `stitch/node/1`. Each frame is a big-endian `u32` length and a postcard-encoded frame, at most 32 MiB; blob content follows its `Blob` frame as raw bytes. Initiator I, responder R:
1. I `Hello { version, repo }` (version 3 since 2026-10-27; 2 since segments are one per turn, still served); R `Welcome`, or `Refused { reason }` (another version, a repository it does not hold). In version 2 each side seals its open segments in the repository before it lists its items; in version 3 the initiator seals when its sync wants it and asked the responder to in its `Summary`.
2. Reconciliation in lockstep: I `Round(messages)`, R `Round(replies)`, until I has nothing to send and sends `Finish`. Replies only answer messages, so R never holds anything back.
3. Both at once: each `File { path, bytes }` the other lacks, then `FilesDone`. The receiver verifies each file and indexes them.
4. Version 2 only (in version 3 each side fetches the blobs it lacks afterwards, with `GetBlobAt` on the same connection): both at once: `WantBlobs` (every blob its records and segments reference that it lacks, if it pulls all; nothing if it fetches on demand), then each blob the other wants as `Blob { hash, size }` and the content, or `size: None`, then `BlobsDone`. Content that does not hash to what was asked is discarded.
5. R `Bye` once it has read everything; I finishes its stream on it, so nothing I sent is still in flight. Either side gives up after 30 seconds without a frame.

Connections are kept, one per peer, and either side opens streams on one whichever dialed it (a node of version 2 serves only connections it accepted).

A summary (version 3) is its own stream: `Summary { version, seal, repos: [(repo, fingerprint)] }`, the fingerprint of each whole repository the initiator holds; answered by `Summaries { version, repos: [fingerprint or None] }` in the same order (`None`: not held). Only repositories that differ get a sync stream. A node of version 2 drops the stream unanswered: it is then synced repository by repository in version 2.

A blob fetch is its own stream: `GetBlob { hash }`, answered by `Blob` and the content; or `GetBlobAt { hash, offset }` (version 3), answered by `Blob` with the whole size and the content from `offset`, so a transfer cut off goes on where it stopped.

A question is its own stream too: `Ask(bytes)`, answered by `Answer(bytes)` (or `Refused` while the node cannot answer yet). The bytes are the node's own encoding: the protocol carries them, the node answers them. The one question so far is a node's models, so a node can delegate with a model the other one has.

Pairing has its own ALPN, `stitch/pair/1`, accepted from any node while an invitation is open (design §2.6). The ticket is `STITCH1` and the upper-case base32 (no padding) of the postcard encoding of `{ id: [u8; 32], secret: [u8; 16], addrs: [SocketAddr] }`, which a QR code holds in its alphanumeric mode. On one stream: the new node `Join { version, secret, name }`; the inviter `Welcome { name, home, fleet }` or `Refused { reason }`. Outside an invitation, the inviter closes the connection with the reason "not accepting new nodes".

**Deleted threads** (design §2.5) are the one thing taken out of the set: a node that holds a `deletion` record removes the files of the threads it names and of the records going with them, and keeps their IDs in `purged` in the record root (local, never synced). A received file of a purged record, of a deleted thread's segments, or of a record that would go with one (by its `thread`, `target`, `answers` or `claims`, or a schedule's `reports-to`) is not written. Two nodes that both honoured the tombstone hold the same set again.

Received files are verified (they parse, and their IDs match their path) before they are written, and are written without a per-file sync to disk: the sender still holds them. A file that a crash leaves incomplete fails verification when the index next reads it, is moved to `.tmp/corrupt/` in the record root, and is fetched again by the next sync. Files a node writes itself are synced to disk, file and directory, before the write returns.

**Nodes** are listed in the home workspace as `capabilities` records, one per node, each superseding the previous one from the same node:

```markdown
+++
v = 1
id = "2026-10-11T132455Z-mbp-c9aa"
kind = "capabilities"
created = 2026-10-11T13:24:55Z
author = { node = "mbp" }
node = "mbp"
always_on = false
power = "battery"
workers = ["claude-code", "codex", "native"]
models = ["llama.cpp:nomic-embed-text-v2"]
repos = ["stitch-home", "stitch", "otter"]
+++
```

## 8. Derived status

Computed by the index from references only; never stored.

| Record | Status | Rule |
|---|---|---|
| task | `open` | no run references it |
| | `running` | a run references it and holds an unexpired claim, or is local and alive |
| | `waiting` | an open question references it or one of its runs |
| | `to-review` | a run has ended with a final report, and no `close` yet |
| | `over-budget` | the `usage` events of its runs exceed its budget |
| | `done`, `dropped`, `failed` | a `close` record closes it |
| question | `open` / `answered` | an `answer` references it |
| finding | `open` → `addressed` → `verified` or `still-failing` | resolutions and verifications reference `<id>#F<n>` (the handoff rules) |
| run | `active` / `asleep` / `ended` | a final report (`final = true`), a live process, or a pending wake trigger |
| assignment | `unclaimed` / `claimed` / `expired` | claims and their `lease_until` |

The attention view (design §2.3) is: open questions, findings `still-failing`, tasks `over-budget`, and failed runs. Each is one index query.

## 9. A worked example

The user, on the phone, asks for a draft of this document in the Stitch repository, with a photo of a whiteboard. The laptop runs Claude Code on it; it asks one question; the user answers; it reports; a second worker reviews.

Files written, in order, in the Stitch project repository:

```
.stitch/
  repo.toml
  records/2026-10/
    2026-10-11T135840Z-phone-a2x8.md     task       (phone)
    2026-10-11T135902Z-phone-h4nn.md     assignment (phone: "a node that has the stitch repo and claude-code")
    2026-10-11T140055Z-mbp-w2kq.md       claim      (mbp)
    2026-10-11T140112Z-mbp-q7cd.md       run        (mbp)
    2026-10-11T143207Z-mbp-k3f9.md       question   (§1 sample)
    2026-10-11T144010Z-phone-t6bd.md     answer     (phone)
    2026-10-11T151530Z-mbp-zz3e.md       report     final = true
    2026-10-11T153300Z-nas-b9dd.md       review     (nas, Codex)
  threads/2026-10-11T140112Z-mbp-q7cd/
    2026-10-11T141530Z-mbp-77ha.jsonl
    2026-10-11T151529Z-mbp-p8vb.jsonl
```

**The task**

```markdown
+++
v = 1
id = "2026-10-11T135840Z-phone-a2x8"
kind = "task"
created = 2026-10-11T13:58:40.551Z
author = { node = "phone", user = true }
workers = ["claude-code"]
budget = { usd = 5.0, hours = 2 }
permissions = { write = ["docs/"], shell = false, network = false }
branch = "task/records-draft"

[[attachments]]
hash = "blake3:9f2c4e…"
type = "image/heic"
name = "IMG_4121.HEIC"
bytes = 2481133
+++

# Draft the records format

Write docs/records.md: encoding, IDs, layout, transcripts, derived status.
Start from Otter's handoff records. The whiteboard photo has the kinds we listed.
```

**The claim**

```markdown
+++
v = 1
id = "2026-10-11T140055Z-mbp-w2kq"
kind = "claim"
created = 2026-10-11T14:00:55Z
author = { node = "mbp" }
lease_until = 2026-10-11T16:00:55Z

[refs]
claims = "2026-10-11T135902Z-phone-h4nn"
+++
```

**The run**

```markdown
+++
v = 1
id = "2026-10-11T140112Z-mbp-q7cd"
kind = "run"
created = 2026-10-11T14:01:12.004Z
author = { node = "mbp" }
worker = { kind = "claude-code", version = "2.4.1", session = "8d1f…", model = "claude-opus-5-5" }
worktree = "task/records-draft"

[refs]
task = "2026-10-11T135840Z-phone-a2x8"
assignment = "2026-10-11T135902Z-phone-h4nn"
+++
```

**The answer**

```markdown
+++
v = 1
id = "2026-10-11T144010Z-phone-t6bd"
kind = "answer"
created = 2026-10-11T14:40:10.882Z
author = { node = "phone", user = true }
choice = "Keep TOML"

[refs]
answers = "2026-10-11T143207Z-mbp-k3f9"
+++

TOML. Strict beats familiar here.
```

**The final report**

```markdown
+++
v = 1
id = "2026-10-11T151530Z-mbp-zz3e"
kind = "report"
created = 2026-10-11T15:15:30.240Z
author = { node = "mbp", run = "2026-10-11T140112Z-mbp-q7cd" }
final = true
commits = ["b41c9e0d7f3a5c21e8d6f0a9b2c4e1d3f5a7b9c0"]

[refs]
task = "2026-10-11T135840Z-phone-a2x8"
run = "2026-10-11T140112Z-mbp-q7cd"
+++

# Records format drafted

docs/records.md is on `task/records-draft`, one commit. It recommends TOML
front matter (answered), time-node-random IDs, a flat month layout and
JSON Lines transcripts.

Not done: the index schema; it belongs with the node code.
```

**The review** (handoff format, findings as sub-items)

```markdown
+++
v = 1
id = "2026-10-11T153300Z-nas-b9dd"
kind = "review"
created = 2026-10-11T15:33:00Z
author = { node = "nas", run = "2026-10-11T152010Z-nas-9kfe" }
reviewed = "b41c9e0d7f3a5c21e8d6f0a9b2c4e1d3f5a7b9c0"

[refs]
task = "2026-10-11T135840Z-phone-a2x8"
reviews = "2026-10-11T151530Z-mbp-zz3e"
+++

# Review: records format draft

## F1 — P2 A `set` on a record the node does not hold
- Location: docs/records.md §6
- Evidence: a `set` lives in its target's repository. A phone that holds only the home workspace cannot rename a task in a project repository it does not hold.
- Suggested direction: allow the `set` in the home workspace with a repository hint on `target`, and let the index apply it wherever the target is.
```

The task is now `to-review` with one open finding; the attention view is empty (no open question). The digest shows the report and the finding. Once a resolution and a verification reference `2026-10-11T153300Z-nas-b9dd#F1` and the user merges the branch and writes a `close`, it is `done`.

## 10. The graph

The records and their references are meant to be rendered as a graph the user explores visually, in 2D or 3D (design §2.3): chats, tasks, runs, agents and what they produced, and how they connect. The format does not draw anything, but it guarantees what a renderer needs:

- **Every relation is an explicit, typed, directed edge** in `[refs]`. Nothing that should appear as an edge lives only in a body. Links in a body (`stitch:<id>`) become `mentions` edges, derived by the index.
- **Edges can target sub-items:** a task spawned from message 12 of a chat references `"<thread>#m12"`, a resolution references `"<review>#F1"`. A renderer can show a thread as one node or open it into its messages.
- **Everything has a time** (`created`, and `at` per transcript event), so time can be an axis, and the graph can be replayed as it grew.
- **Everything has an author** (node, user or run), so the graph can be grouped or coloured by device, worker kind or agent.
- **Derived edges are not stored:** engram associations (design §2.9), similarity from embeddings, mentions. The index computes them, and a renderer shows them as suggested, distinct from the edges people and agents wrote.
- **Layout is never in records.** Positions are computed. If pinned positions or saved views are wanted later, they are view state (a `view` record or local to a node), decided with the renderer.

The index keeps the graph as an edge table (source, relation, target, time, author) next to the node table, which is what a renderer queries.

## 11. Later

Not needed to start:
- signing records (depends on design open question 3, multi-user);
- the index schema;
- a `set` on a target in a repository the node does not hold (the sample finding in §9).
