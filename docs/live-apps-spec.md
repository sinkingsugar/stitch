# Spec: apps over the node's live state (the dashboard, and approvals from apps)

Status: **steps 1–2 built** (2026-10-17): `stitch.live.threads`, `attention`, `jobs` and `on`, consent per app, and the built-in dashboard (`stitch dashboard`) are in `docs/design.md` §2.10 ("Live state for apps") and no longer here. What is left (steps 3–5) is a proposal: when built, its decisions move into `docs/design.md` (§2.10 apps, §2.3 approvals, §2.7 threads) in the same commit, and this file goes.

## Why

The first dashboard (`apps/dashboard/`, 2026-10-17) was an HTML app whose data an agent collected by running `threads`, `gh run list`, `git status` and the like, and wrote into `data.json`. That design fails three ways:
- **Stale at once.** It's a snapshot, and reloading shows the same one.
- **Costs a model call per refresh,** and a pinned app couldn't even ask for one: reports from pinned apps were refused, which is being fixed separately.
- **Wrong facts.** The agent summarized old thread reports instead of the repositories, and called finished work unfinished.

The node already knows the facts: threads (`threads` tool), jobs (`jobs`), pending approvals (the attention list), places and their checkouts. The node should serve them live. The page renders them, and an agent adds only judgement.

## 1. Live state for apps: what is left

Built (design §2.10): `live.threads`, `live.attention`, `live.jobs`, `live.on`, GETs under `/_stitch/live/`, changes through the long poll. Still to come, the same way:

| Call | Returns | From |
|---|---|---|
| `stitch.live.places()` | each place: label, kind, path on this node; for a git checkout: branch, ahead/behind its upstream (from the last fetch, never fetching on a read), uncommitted count, worktrees (path, branch, clean or dirty) | `place.rs`, gix (already a dependency) |
| `stitch.live.ci({place})` | latest runs of the checkout's GitHub repo: title, sha, status, conclusion (with `cancelled` distinct from `failure`), url | the node's GitHub client (`github.rs`), cached per place for 60 s and refreshed in the background when an app is open, never once per page load |

- `on` gains `places` and `ci`.
- Phone: `places` lists its clones; `ci` goes through the node like any other GitHub call.
- Under the same consent as the rest of `live`.

## 3. Answering approvals from an app (was app-answer-spec.md)

- `stitch.answer(question, "allow" | "always" | "deny", reason?)` goes to `Request::Answer { question, allow, always, reason }`, the same path as the bell, the TUI and `stitch answer`.
- **Only the user answers.** It works only inside a user gesture:
  - The client hosting the page (iOS web view, desktop browser) mints a one-time token on a real tap or click.
  - The bridge sends it, and the node refuses an answer without a fresh token.
  - `always`, and anything outside the sandbox, gets the client's own native confirmation with the exact command, never page HTML a page could fake.
- A separate opt-in from `live`, also off by default.
- The answer record says `via: app <name>`.
- Works from pinned apps (it goes to the node, not a conversation).
- Desktop browser: no native confirmation is possible there, so `always` and outside-sandbox answers are refused, and plain allow/deny need the gesture token from the page's own click (the browser's `userActivation`, checked by the bridge). That is weaker than on the phone, so say so in the prompt that grants it.

## 4. Insights: the agent's part, optional

**Superseded in part (2026-10-29): loose ends** (design §2.10) are built from this: the quick model's judgement, per conversation, of whether it waits on the user, kept as a `set` on the thread and shown next to the facts. Unlike what follows, it **runs hourly on the node's timer, behind a free check** (no model call unless a conversation stopped without a judgement of its latest step), as well as on demand (the dashboard's button, `stitch loose-ends`). Efforts and the summary below stay a proposal.

- An agent (the dashboard thread, Sonnet) writes judgement into the app's state (`stitch.storage`, key `insights`):
  - `efforts`: named groups of thread IDs, commits and places, each with a one-line status;
  - `summary`: one paragraph;
  - `flags`: things that need the user that facts alone don't show, e.g. "the stitch-otter worktree is obsolete";
  - `at`: when it was written.
- The page shows insights next to live data, with its age, and never instead of it. A fact always comes from `live`.
- The agent is woken rarely: when a spawned thread reports, by the Refresh button (`stitch.report`, which works from pinned apps once the pinned-apps change lands), or by the user. Not on a timer.
- It reads the same `live` data through its tools (`threads`, `jobs`), so it never scrapes.

## 5. The dashboard: what is left

Built (design §2.10): the template in the binary, `stitch dashboard`, the phone's offer, sections 1–3 and the jobs and feed. To add as the steps below land:
- "Needs you": Allow/Deny on each approval (§3), failed (not cancelled) CI.
- Efforts (from insights, §4).
- Repos and CI per place (step 3).
- Clean-up (optional), collapsed: stray worktrees, dirty old checkouts (step 3).
- `apps/dashboard/` (the agent-written prototype, untracked) is superseded and can be deleted.

## Tests (left)

- **`places` and `ci`** against a fixture node: a dirty checkout with a worktree; CI with a cancelled run.
- **`answer`:**
  - outside a gesture: refused;
  - with a gesture: reaches `Request::Answer`;
  - `always`: native confirmation;
  - from a browser: `always` refused;
  - the record carries `via`.
- **UI test:** the built-in dashboard on a fixture shows a pending approval, and tapping Allow answers it.

## Order

1. ~~`live.threads`, `live.attention`, `live.jobs`, `live.on`, with consent.~~ Built.
2. ~~The built-in dashboard template on them, with `stitch dashboard`.~~ Built.
3. `live.places` and `live.ci`.
4. `stitch.answer` (§3).
5. Insights (§4), with the dashboard thread turned into the insights writer. (Loose ends built 2026-10-29: design §2.10.)
