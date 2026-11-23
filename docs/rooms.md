# Scopes and rooms: design

Status: decided 2026-11-13, with the user, after surveying edge talk's group chat (`2026-11-13T140533Z-mbp-zniw`) and reviewing the first proposal (`docs-rooms`, PR #13) against Stitch's existing digests, artefact index and map (§2.25) and provider replay rules (§2.8). Summary in `docs/design.md` §2.27.

---

## 1. Background: edge talk's group chat and its limits

Edge talk's group chat (`Scripts-Src/db.sql:59-83`, `GroupChatViewModel.swift`, `ChatViewModel.swift:1338-1431`, ~80 commits from 2026-06-19) modelled a group as a `kind='group'` conversation over personas (`ChatActor`: name, prompt, model, tools, temperature, TTS voice; at least 2):

1. **Blind parallel rounds:** every persona answered each user message in parallel and saw the others' answers only one round later, inside a rebuilt `<chat><participant>` preamble. Real back-and-forth within a round was impossible.
2. **N× cost per message, user-driven:** everyone spoke every round unless `@`-targeted; no `[pass]` and no bounded handoff.
3. **Final text only, unbound prompt growth:** each member's prompt was reconstructed from scratch every round, dropping tool work and growing without per-member compaction.
4. **Foreground-only on one device:** ran on the phone's main actor (`supportsBackgroundSend: false`), shared one device's tools, suppressed member notifications without sending a group notification, and disabled voice calls, queue, memories, audio/video attachments and truncate.
5. **Shape bugs:** personas speaking for one another (`77769346`), group memories leaking across personas (`2f8af52e`), fork projections observed twice into memory (`412bd897`, #33), and untitled-group crashes (`a1241011`).

---

## 2. How scopes and rooms relate (and why they are decoupled)

Stitch has no personas (§2.9): **a participant is a model plus a context**:
- **A fresh model** (`read_only: true` by default, or `tools: false`): an independent second opinion or council member.
- **An existing thread** with its full working history and warm cache: a colleague who did the work.

Meanwhile, §2.25 already built the three foundations that the earlier draft assumed were missing:
- **Incremental message-anchored `digest` records** (`stitch-node/src/digests.rs`, `DIGEST_KIND`): `headline` and up to 12 anchored items (`goal`, `outcome`, `decision`, `finding`, `change`, `question`, `ask`) refreshed after turns on the quick model under a $2/day cap, **including for places with `memory = false`**.
- **The deterministic `artefacts` index** (`stitch-store/src/artefacts.rs`): `path`, `url`, `commit` and `thread` edges extracted per segment on every node.
- **Efforts on the 2D/3D map** (`apps/client/src/map/`): compaction chains folded with their recursive spawn trees in Islands (2D) and Space (3D).

Because segments, digests and artefacts are already synced to every node, **pointing a thread at a piece of the map (a scope) is primarily a local read lens, not a live multicast to wake N cold threads**:
- Waking 25 old threads in an effort whose provider prompt caches expired days ago would re-prefill 25 transcripts at full input price and stall on offline nodes, when reading their synced `digests`, `artefacts` and anchored messages costs $0 and milliseconds.
- Asking a live thread via `send` + `wait` (which weave already has) stays a surgical Layer-4 escalation for the 1–2 threads whose warm context or private reasoning is needed.

Scopes and rooms therefore **do not block on a shared multicast core**: scopes layer directly onto §2.25 first, rooms build on real participant threads with append-only delta relay second, and they meet cleanly when a thread or map selection is invited into a room.

---

## 3. Thread and effort scopes

Instead of pasting `<convo>` text into a chat or relying on ambient `recall` across all ~1,000 threads, the user attaches one or more **scope targets** to a message or a new thread:
- `effort:<root-thread-id>` — a conversation's compaction chain plus every thread spawned under it (matching `map/model.ts`'s effort roll-up).
- `thread:<thread-id>` — one conversation's compaction chain alone.

### Storage and prompt caching (`Part::Scope`)

- `Request::Chat` and `Request::Send` (and `spawn`) accept `scopes: Vec<ScopeTarget>` (`{ kind: Effort | Thread, id: RecordId }`).
- When the user message is written, the node resolves the targets against the local index and writes a **`Part::Scope { scopes, briefing }`** part alongside the message's text (like `Part::Spoken` and `Part::Recalled`, `crates/stitch-records/src/segment.rs`).
- **Immutable once written:** storing the rendered `briefing` snapshot inside `Part::Scope` keeps every subsequent step's and turn's prompt bytes identical, preserving provider prompt caches and Claude's signed thinking block bindings (§2.8) even after scoped threads later update their digests. Older builds deserialize `Part::Scope` as `Part::Other` and skip it (`STORE_FORMAT` unchanged, `STORE_SHAPE` bumped).
- **UI rendering:** the web client and iOS app strip `Part::Scope`'s prose from the user bubble and show compact, clickable **scope chips** (`⎇ Effort: <title> (N threads)` or `⎇ <title>`) that open the effort on the map or the thread.

### Four layers (read-first, wake-last)

1. **Layer 1 — Instant briefing (`Part::Scope`, zero model calls):**
   Assembled from SQLite (`Index`) + segments in milliseconds, capped at ~20 KB:
   - Topology: each thread's ID, title, place, runner node, model, state, cost, timestamps, parent/compaction links, and an explicit **`[memory: off]`** marker whenever the thread or its place has `memory = false` (§2.25).
   - Each thread's existing `digest` (`headline`, freshness, and anchored `[kind] text (<thread>#<msg>)` items); when a short or brand-new thread has no digest yet, its goal and latest reply preview. On large scopes (many leaf threads), roots and high-signal threads keep full digest items while mechanical leaves fold to one-line headlines with instructions to drill down via `read_thread`.
   - Deduped `artefacts` across the scope (files written/read, commits, URLs).
2. **Layer 2 — Scoped `recall` and deterministic search:**
   - `recall` gains an optional `scope: Vec<String>` (`effort:<id>` or `<thread-id>`).
   - Resolves the scope's thread IDs (including compaction chains and, for `effort:`, recursive descendants) and restricts both Engram's EmbeddingGemma 2 vector/associative search **and** deterministic segment/artefact keyword matching to that thread set—so `memory = false` places remain searchable within a scope while reporting which threads have memory off.
3. **Layer 3 — Drill-down (`read_thread` tool in `weave`):**
   - A read-only built-in tool (`read_thread`) available in normal and `read_only` threads on every node: given `thread` (or an anchor `<thread>#<msg>`), returns its metadata (`memory: off` included), latest `digest`, `artefacts`, and a bounded window of transcript messages (`around`, `from`, `limit`, `include_tools: bool`), redacted through `ToolRunner::redact`.
4. **Layer 4 — Ask a live thread (`send` + `wait`):**
   - Existing weave `send` when the agent needs to question a live colleague thread that still holds private tool output or warm context.

---

## 4. Rooms (multi-model and multi-thread group chat)

### Architecture: a room of real threads

A room is a thread record (`kind = "thread"`) with `room = true`. Its own segments hold only the shared conversation: the user's messages (including attachments and `Part::Scope` chips) and each participant's final replies (`Role::Assistant`, `from = "participant:<slug>:<thread_id>"`, `model = "<provider/model>"`).

Each participant is an ordinary Stitch thread:
- **Why real threads (Option A) instead of one-thread-many-voices (Option B):**
  - **Provider native replay and prompt caching (§2.8):** Claude 5.x binds signed thinking blocks to the exact prefix, Gemini 3 validates `thoughtSignature`s, OpenAI/Codex replays encrypted reasoning items routed by `session-id`/`prompt_cache_key`, and DeepSeek/Kimi require `reasoning_content` round-trips. Interleaving providers in one transcript forces cross-provider fallbacks and cache misses on every turn. Giving each participant its own thread keeps 100% native replay and warm caches per model.
  - **Per-participant isolation:** each participant has its own model, node, place, `read_only` / toolset rules, approvals and compaction chain, and existing threads can join with all their prior work intact.
- **Defaults for fresh models:** adding a model to a room spawns a participant thread under the room with **`read_only = true`** by default (or `tools = false` when chosen), `remember = false` and `recall = true`. Two room participants never get concurrent writable shells in the same checkout by accident. Invited worker threads keep their own checkout/mode (or can be muted/set read-only by the user).
- **Read-only asks, in a room** (decided 2026-11-14, after room `2026-11-14T095653Z-iphone-6dkd`, where a read-only participant wrote a whole app and was refused `write_file`): the user is in the room, so a read-only participant's write or refused command in its room turn is an **approval question** ("Fable (read-only, in room “…”) wants to write …"), not a failure: **allow** runs that call as a writable thread's, **always** makes the participant writable (a `set` of its `approval` for the user, synced), **decline** refuses as before, with the reason. It asks the same when its turn is relayed to the node it runs on by another node that runs the room (2026-11-17). Out of a room, read-only stays refused without asking (an agent's survey threads never block on the user). The user switches a participant between read-only and writable from the room itself (the iOS roster row's menu and swipe, the web client's `ro`/`rw` pill). The clients pick **Read-only (asks before writing)** (default), **Can write** or **No tools** for each model added. Details in design §2.27.
- **No buttons in a room** (2026-11-21, issue 2026-11-21T070413Z-gamer-58iv): `suggest_replies` and `offer_actions` render from a thread's own transcript, which nobody reads for a participant (the room only gets its reply text), so a participant's call is refused ("does not show in a room … ask in words") rather than claiming buttons were shown.
- **Questions in the room** (2026-11-21): a participant's pending approval (a read-only ask, or any approval of its turn) shows in the room itself, not only on its own thread: an inline Allow / Always / Decline card with the speaker's header and "Fable wants to write …", the working line "Fable is waiting for your approval", the needs-you mark on its roster pill (web and iOS). Once answered or withdrawn, the asking node writes a quiet note into the room, without a model call (`[Fable was allowed to write …]`, `[Fable was not allowed to write …: <reason>]`, `[Fable no longer asks to …]`): an event, not a message, so it is never relayed to the participants nor observed by memory. Details in design §2.27.
- **Roster and policy (`set` records on the room):**
  - `roster`: ordered list of `{ slug, name, thread, muted }` (`slug` is unique in the room for `@slug` mentions, e.g. `opus`, `gemini`, `gpt`).
  - `room-policy`: `{ mode: "sequential" | "parallel", hop_cap: u32 }` (defaults: `mode = "sequential"`, `hop_cap = 2`).

### Stateless, cache-safe delta relay (`Part::RoomTurn`)

- Before participant `P` takes a turn, the room coordinator scans `P`'s own transcript for the latest `Part::RoomTurn { room, upto, .. }` part and collects every message in `room` strictly after `upto` (excluding `P`'s own replies already generated by `P`).
- Those new room messages are appended to `P` as a single `Role::User` message formatted cleanly with speaker headers (`[Ann]: …`, `[Opus · anthropic/claude-opus-5-5]: …`, scope briefings and blob attachments carried by `BlobHash`), accompanied by `Part::RoomTurn { room, upto: latest_room_msg_id, slug, roster_summary, can_pass, note: ROOM_NOTE_NUMBER }`.
- **Crash-safe and multi-node safe:** because `upto` lives in `P`'s own append-only segment, no separate cursor table can drift across syncs or restarts. Versioned `note` numbers (like `Part::Spoken.note`, §2.26) guarantee historical prompt bytes never change if the room instruction wording is refined later.
- **Memory single-observation rule (edge talk #33):** Engram observes the room thread once. Fresh participant threads have `remember = false`, and on invited remembered threads Engram skips `Part::RoomTurn` exchanges so room messages and replies are never observed twice.

### Turn-taking policies

1. **Targeting (`@slug`):** if the user's message mentions `@slug` (or `@Name`), those participants are scheduled in mention order; otherwise all unmuted participants run in roster order.
2. **Sequential with `[pass]` (default):**
   - Participants run one at a time. As soon as speaker 1 finishes, its reply (the text of its whole turn, before and after its tool calls, and any generated/attached images) is written to the room's segment and pushed live to clients; speaker 2 then receives speaker 1's reply in its same-round delta.
   - A participant's turn the room did not start (woken by a report of a thread it spawned, a schedule) is written to the room too when it ends, as its room replies are; the others get it in their next delta, its author never.
   - Unmentioned participants in a sequential round (`can_pass = true`) are told they may reply `[pass]` when earlier speakers already covered their point. `[pass]` (and `[nothing new]`) writes no noisy bubble to the room transcript (clients show a quiet "passed" status for the round). Explicitly `@`-mentioned participants always answer.
3. **Parallel blind ("Council" toggle):**
   - For independent opinions without sycophantic anchoring (as in §2.25's initial 3-model review): all scheduled participants receive the pre-round delta concurrently and run in parallel without seeing one another's in-flight answers. Round 2 delivers everyone's round-1 replies in the next delta.
4. **Bounded `@handoff` (off unless explicitly mentioned, capped at `hop_cap` = 2):**
   - Unprompted AI-to-AI auto-continue is **off** (prevents polite agreement loops). In sequential mode, if a participant's reply explicitly mentions `@other_slug` in the roster, the coordinator schedules that participant, up to `hop_cap` AI-to-AI hops per user message (default 2, `0` disables handoffs), stopping immediately on `[pass]`, user interjection/stop, or budget exhaustion.
   - **Everyone gets a first say before anyone a second** (decided 2026-11-14, after a real room went gemini → fable → gemini → fable → gpt: "Broadly agree with @gemini" and "Good catches from @fable" each handed the floor back, and gpt spoke last, its real question to fable unanswered with the cap spent). A participant named who is still to speak this round keeps its place in the queue (it must answer, no `[pass]`) and costs no hop; one who already spoke is queued **after** everyone still waiting, not in front. Room note 1 (`ROOM_NOTE_NUMBER = 1`) tells participants to refer to each other by name and write `@handle` only to ask something they need answered; turns written under note 0 keep its wording, so their replayed history is unchanged.

### Frictionless "Pull in model…" from any 1:1 chat

Users rarely know up front that a chat needs a group. From any ordinary 1:1 thread `T`, **"Pull in model…"** (in the web client, iOS chat menu, or CLI) creates a room `R` in `T`'s place:
1. Invites `T` itself as Participant #1 (`T`'s own transcript, native blocks and warm cache stay untouched!).
2. Spawns the newly chosen model(s) (`read_only: true` by default) as Participant #2+, seeding their first turn with `T`'s `Part::Scope` briefing (and recent exchange preview) so they are immediately caught up without copying or corrupting `T`.
3. Opens `R` (with a back-link to `T`).

---

## 5. Clients and voice calls

### Web client (`apps/client`, §2.24 / §2.25)

- **Map scopes (`apps/client/src/map/`):**
  - Effort and thread cards in Islands (2D) and Space (3D) gain **"Ask about this"** (opens a new chat or attaches to the composer) and **"Open Room"**.
  - `Shift+click` (or "+ Select") builds a multi-selection scope tray on the map ("3 efforts · Ask / Open Room / Clear").
- **Composer and transcript (`apps/client/src/transcript/`):**
  - `+ Scope` button in `Composer.tsx` to search/pick threads or efforts; removable scope chips above the input; clickable scope chips on sent user bubbles.
  - `@slug` autocomplete in rooms.
- **Rooms UI:**
  - `+ Room` next to new chat, plus **"Pull in model…"** in `ThreadMenu.tsx`.
  - Roster bar at the top of a room: participant pills (accent dot, name, `provider/model`, runner node, spend, mute toggle, click to inspect participant's private tool thread, `+ Model / + Thread`), `Sequential | Council` policy switch, and per-participant / room-wide Stop.
  - Merged transcript with speaker header badges (`Name · model` + link to the participant's step when tools ran) and live working status showing which participant(s) are thinking or running tools.

### iOS app and voice calls (`apps/ios`, §2.26)

- **Scopes & Rooms in `ChatView` / `RootView`:**
  - Room icon badge in the thread list; participant threads nested under the room in `ThreadTree`.
  - Scope chips on user bubbles + attach thread/effort in composer; author headers on room bubbles (tap header → participant's private thread); live status per active participant + Stop; Roster sheet (add model, invite thread, mute, `Sequential | Council` toggle, hop cap, per-participant spend); `@` mention bar; **"Pull in model…"** in chat menu; `room` fixture for `just ui-shot room`.
- **Voice calls in a room (`CallController`):**
  - Forces sequential turn-taking while a call is active unless council is explicitly asked for; queues completed participant replies in turn order so voices never overlap; picks distinct voices per participant (`VoiceInfo.pick`, announcing the speaker's name when the speaker changes); treats spoken participant names as `@mentions`; barge-in clears the speech queue and `"stop"` cancels all active room turns.

---

## 6. Build order

Each step lands as a clean, tested slice (`just quick`, plus `just check-apple` on `mbp` whenever crates linked by iOS change):

1. **Step 1 — Node & CLI: Scopes and `read_thread`:** `Part::Scope` in `stitch-records` (`STORE_SHAPE` updated, `STORE_FORMAT` unchanged); scope target resolution (`effort:<id>`, `thread:<id>`); Layer-1 briefing builder over `digests` + `artefacts` with `[memory: off]` markers and size cap; `read_thread` tool in `weave`; `scope` filter in `recall` (Engram + deterministic segment/artefact search); `scopes` on `Request::Chat`, `Request::Send`, `spawn`, and CLI; Rust unit and node integration tests.
2. **Step 2 — Web client: Scopes in Composer and 2D/3D Map:** `just client-types`, scope chips in `Composer.tsx` and `Rich.tsx`, thread/effort picker, Effort/Thread card "Ask about this" and `Shift+click` multi-select scope bar in Islands & Space (`apps/client/src/map/`), mock fixtures and `bun test` suite.
3. **Step 3 — Node & CLI: Rooms coordinator and "Pull in model":** `room` thread creation and 1:1-to-room promotion ("Pull in model…"), `roster` and `room-policy` `set`s, `Part::RoomTurn` stateless delta relay (versioned note, attachments by hash), Engram single-observation rule, Sequential (`[pass]` + `@handoff` cap) and Parallel Council execution, cancellation and budget enforcement, CLI (`stitch room …`) and Rust integration tests against mock models.
4. **Step 4 — Web client: Rooms UI and Map → Room:** `+ Room`, "Pull in model…" in `ThreadMenu`, Roster bar, speaker headers and private-thread links in `Rich.tsx`, `@slug` autocomplete in `Composer.tsx`, multi-participant working status & stop, "Open Room" from Map cards/selection, mocks and `bun test` tests.
5. **Step 5 — Apple (`mbp`): `check-apple`, iOS Scopes & Rooms UI, Voice Calls, `ui-shot`:** `StitchFFI` bindings, iOS scope chips, room transcript headers, Roster sheet, "Pull in model…", `@` mentions, sequential multi-voice `CallController` queue, `just ui-shot room`, and `just check-apple` + simulator build/tests.

---

## 7. Noted separately: Apple Watch app (open brainstorm, not part of this build)

Kept as an open item in `docs/design.md` §4:
- Answering approvals and short questions from the wrist;
- Starting or continuing a voice call (§2.26) into a thread or room;
- Glanceable status (threads working, needs-you count, spend today, latest report).
