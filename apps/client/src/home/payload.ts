// Payloads of `Request::Live` (apps/live.rs, schedules.rs, loose.rs build them
// with `json!`, so these are written by hand to match; see ../payload.ts).

export interface Question {
  id: string;
  title: string;
  thread: string | null;
  thread_title: string | null;
  node: string;
  place: string | null;
  session: string | null;
  command: string | null;
  approval: boolean;
  grant: string[];
  asked: string;
}
/** A thread that waits on the user without an approval of its own (`needs_threads`
 * in apps/live.rs): `blocked` or `reported-blocked`, already minus a spawned
 * thread's report its parent has and what the user dismissed. */
export interface NeedThread { id: string; title: string; why: "blocked" | "reported-blocked"; said: string; parent: string | null; since: string | null }
/** `threads` is absent from a node older than local-52's batch. */
export interface Attention { node: string; at: string; questions: Question[]; threads?: NeedThread[] }

export interface LiveJob {
  running: boolean;
  job?: number | null;
  session?: string | null;
  thread?: string | null;
  thread_title?: string | null;
  condition?: string | null;
  how?: string | null;
  command?: string | null;
  file?: string | null;
  last_lines?: string | null;
  started?: string | null;
  ended?: string | null;
}
export interface Jobs { node: string; at: string; jobs: LiveJob[] }

export interface Schedule {
  id: string;
  name: string;
  goal: string;
  when: string;
  at: string | null;
  node: string;
  place: string | null;
  state: string;
  paused_by_node: boolean;
  self_wake: boolean;
  thread: string | null;
  thread_title: string | null;
  latest_thread: string | null;
  latest_title: string | null;
  check: string | null;
  read_only: boolean;
  budget: { usd: number | null } | null;
  daily_cap: number | null;
  today: number | null;
  next: string[];
  firing: boolean;
  here: { last: string | null; last_said: string | null; fired: number; failures: number } | null;
}
export interface Schedules { node: string; at: string; schedules: Schedule[] }

export interface LooseItem {
  thread: string;
  title: string;
  kind: "conversation" | "thread";
  verdict: string | null;
  line: string | null;
  judged: string | null;
  since: string;
  unread: boolean;
  node: string;
  place: string | null;
  /** What auto-handling did (or would do) on it; absent from older nodes. */
  auto?: LooseAuto | null;
}
/** `loose-end-auto`: `outcome` is continued, dismissed, would-continue,
 * would-dismiss, declined (the confirmation said no), held (a guard), failed. */
export interface LooseAuto {
  outcome: string;
  act: string | null;
  message: string | null;
  why: string | null;
  mode: string | null;
  at: string | null;
}
export type AutoMode = "off" | "dry-run" | "on";
/** Auto-handling as this node applies it: `here` is its `[loose_ends] auto`
 * (it wins), `synced` the user's setting, `rules` the user's rules. */
export interface AutoSetting {
  mode: AutoMode;
  synced: AutoMode;
  here: AutoMode | null;
  rules: string;
}
export interface LooseEnds {
  node: string;
  at: string;
  checking: boolean;
  unjudged: number;
  last_pass: { at: string; model: string | null } | null;
  /** Absent from nodes older than local-53. */
  auto?: AutoSetting | null;
  items: LooseItem[];
}

export interface LiveData {
  attention: Attention | null;
  jobs: Jobs | null;
  schedules: Schedules | null;
  loose: LooseEnds | null;
}
