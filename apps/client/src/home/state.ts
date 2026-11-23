// The dashboard's live data as signals (Request::Live), and what is derived
// from threads + live data: the "needs you" set and the spawn trees.
import { computed, effect, signal } from "@preact/signals";
import type { Api } from "../conn";
import type { ThreadItem } from "../model";
import { coalesced } from "../coalesce";
import { homeView, setSource, src, threads } from "./source";
import type { Attention, AutoMode, AutoSetting, Jobs, LooseEnds, Schedules, Question, LooseItem } from "./payload";

export const attention = signal<Attention | null>(null);
export const jobs = signal<Jobs | null>(null);
export const schedules = signal<Schedules | null>(null);
export const loose = signal<LooseEnds | null>(null);
export const checking = signal(false);
export const liveError = signal<string | null>(null);
/** Dismissed here before the node says so (thread@since). */
const dismissed = signal<Set<string>>(new Set());

/** The web client's source: the WebSocket, and the user's reach (the store's
 * `select` and `answer`). The dashboard sets its own (dashboard/source.ts). */
export function initLive(a: Api, user: { open(id: string): void; answer(q: string, allow: boolean, always: boolean): Promise<unknown> }) {
  setSource({
    live: (what) => a.live(what),
    looseEnds: (op) => a.looseEnds(op),
    open: user.open,
    answer: user.answer,
    looseAuto: (set) => a.looseAuto(set),
    schedule: (input) => a.schedule(input),
  });
  a.onState((s) => { if (s === "open") void refreshLive(); });
  // The home view shown reads its four lists at once.
  effect(() => { if (homeView.value) void refreshLive(); });
  setInterval(() => void refreshLive(), 60000); // times move on
}

/** A live section the node pushed whole as it moved (design §2.24). */
export function setSection(what: string, value: unknown) {
  if (what === "attention") attention.value = value as Attention;
  else if (what === "jobs") jobs.value = value as Jobs;
  else if (what === "schedules") schedules.value = value as Schedules;
  else if (what === "loose-ends") {
    loose.value = value as LooseEnds;
    checking.value = (value as LooseEnds).checking;
  }
}

async function soft<T>(what: string): Promise<T | null> {
  try {
    return await src.live<T>(what);
  } catch (e) {
    liveError.value = String(e instanceof Error ? e.message : e);
    return null;
  }
}

/** What is read now: the home view shows all four lists; a thread open
 * needs its jobs (the side panel) and the node's name once (`attention`). */
function wanted(): string[] {
  if (homeView.value) return ["attention", "jobs", "schedules", "loose-ends"];
  return attention.value ? ["jobs"] : ["attention", "jobs"];
}

/** At most one round in flight and one queued behind it. */
export const refreshLive = coalesced(async () => {
  liveError.value = null;
  const w = wanted();
  const [a, j, s, l] = await Promise.all([
    w.includes("attention") ? soft<Attention>("attention") : null,
    w.includes("jobs") ? soft<Jobs>("jobs") : null,
    w.includes("schedules") ? soft<Schedules>("schedules") : null,
    w.includes("loose-ends") ? soft<LooseEnds>("loose-ends") : null,
  ]);
  if (a) attention.value = a;
  if (j) jobs.value = j;
  if (s) schedules.value = s;
  if (l) { loose.value = l; checking.value = l.checking; }
});

export const looseItems = computed<LooseItem[]>(() =>
  (loose.value?.items ?? []).filter((i) => !dismissed.value.has(i.thread + "@" + i.since)));

export async function dismissLoose(i: LooseItem) {
  dismissed.value = new Set(dismissed.value).add(i.thread + "@" + i.since);
  try {
    await src.looseEnds({ dismiss: i.thread });
  } catch (e) {
    const d = new Set(dismissed.value);
    d.delete(i.thread + "@" + i.since);
    dismissed.value = d;
    liveError.value = "Dismiss: " + String(e instanceof Error ? e.message : e);
  }
}

export async function checkLoose() {
  checking.value = true;
  try {
    await src.looseEnds({ check: true });
  } catch (e) {
    liveError.value = "Check: " + String(e instanceof Error ? e.message : e);
  }
  await refreshLive();
  checking.value = false;
}

/** Auto-handling as last read or set; a set shows at once, then the node's word. */
export const autoSetting = computed<AutoSetting | null>(() => autoSet.value ?? loose.value?.auto ?? null);
const autoSet = signal<AutoSetting | null>(null);
effect(() => { void loose.value; autoSet.value = null; });
export const autoSaving = signal(false);

export async function setAuto(set: { mode?: AutoMode; rules?: string }): Promise<boolean> {
  autoSaving.value = true;
  const before = autoSetting.value;
  if (before) autoSet.value = { ...before, ...(set.mode ? { mode: before.here ?? set.mode, synced: set.mode } : {}), ...(set.rules != null ? { rules: set.rules } : {}) };
  try {
    if (!src.looseAuto) throw new Error("change it in the Stitch client or with `stitch loose-ends --auto`");
    autoSet.value = await src.looseAuto(set);
    return true;
  } catch (e) {
    autoSet.value = null;
    liveError.value = "Auto-handling: " + String(e instanceof Error ? e.message : e);
    return false;
  } finally {
    autoSaving.value = false;
    void refreshLive();
  }
}

export const questions = computed<Question[]>(() => attention.value?.questions ?? []);

/** What the user dismissed here, before the node says so (ids). */
const hidden = signal<Set<string>>(new Set());

/** Threads that wait on the user without an approval of their own: the node's
 * `attention.threads` (a spawned thread's blocked report its parent has is not
 * the user's; a dismissed one stays gone until the thread moves). A node that
 * does not say falls back to the thread list's states. */
export const blocked = computed<ThreadItem[]>(() => {
  const need = attention.value?.threads;
  const all = threads.value;
  if (need) {
    const byId = new Map(all.map((t) => [t.id, t]));
    return need.filter((n) => !hidden.value.has(n.id)).flatMap((n) => byId.get(n.id) ?? []);
  }
  const asked = new Set(questions.value.map((q) => q.id));
  return all.filter(
    (t) => !hidden.value.has(t.id) && ((t.state === "blocked" && !(t.question && asked.has(t.question.id))) || (t.state === "idle" && t.outcome === "blocked")),
  );
});
export const stalled = computed(() => (schedules.value?.schedules ?? []).filter((x) => x.paused_by_node && !hidden.value.has(x.id)));
/** The badge: everything that waits on the user. */
export const needsCount = computed(() => questions.value.length + blocked.value.length + stalled.value.length);

async function hide(id: string, what: string, f: () => Promise<unknown>) {
  hidden.value = new Set(hidden.value).add(id);
  try {
    await f();
  } catch (e) {
    const d = new Set(hidden.value);
    d.delete(id);
    hidden.value = d;
    liveError.value = what + ": " + String(e instanceof Error ? e.message : e);
  }
  await refreshLive();
}
/** Dismiss a blocked thread: recorded as the user's (`LooseEnds { dismiss }`, synced), so it
 * clears on every device; a new step of the thread brings it back. */
export const dismissThread = (id: string) => hide(id, "Dismiss", () => src.looseEnds({ dismiss: id }));
/** Dismiss a stalled schedule: the user pausing it (it stays paused, no longer "by the node"). */
export const dismissSchedule = (id: string) => hide(id, "Dismiss", () => src.schedule!({ action: "pause", schedule: id }));
export const resumeSchedule = (id: string) => hide(id, "Resume", () => src.schedule!({ action: "resume", schedule: id }));

// ---- spawn trees
/** `under`: a pinned spawned thread lifted out of its parent's tree, the parent's title. */
export interface TreeNode { t: ThreadItem; kids: TreeNode[]; under?: string }

/** The spawn trees, pinned first, then by the latest activity in each tree.
 * A pinned thread is a root of its own, whatever spawned it: nested, a folded
 * parent hid it and the parent's place in the list decided its own. */
export const forest = computed<TreeNode[]>(() => buildForest(threads.value));
export function buildForest(list: ThreadItem[]): TreeNode[] {
  const byId = new Map(list.map((t) => [t.id, t]));
  const nodes = new Map<string, TreeNode>(list.map((t) => [t.id, { t, kids: [] }]));
  const roots: TreeNode[] = [];
  for (const t of list) {
    const n = nodes.get(t.id)!;
    // A parent outside the list (an old conversation) makes a root; so does a cycle.
    const nested = t.parent && t.parent !== t.id && byId.has(t.parent) && !cycles(t, byId);
    if (nested && !t.pinned) nodes.get(t.parent!)!.kids.push(n);
    else {
      if (nested) n.under = byId.get(t.parent!)!.title || t.parent_title || "a conversation";
      roots.push(n);
    }
  }
  const latest = new Map<TreeNode, string>();
  const late = (n: TreeNode): string => {
    let l = latest.get(n);
    if (l === undefined) {
      l = n.kids.reduce((m, k) => { const x = late(k); return x > m ? x : m; }, n.t.last_step ?? "");
      latest.set(n, l);
    }
    return l;
  };
  const order = (a: TreeNode[]) => a.sort((x, y) => +!!y.t.pinned - +!!x.t.pinned || late(y).localeCompare(late(x)));
  const sortAll = (a: TreeNode[]) => { order(a); a.forEach((n) => sortAll(n.kids)); };
  sortAll(roots);
  return roots;
}
function cycles(t: ThreadItem, byId: Map<string, ThreadItem>): boolean {
  const seen = new Set([t.id]);
  for (let p = byId.get(t.parent ?? ""); p; p = byId.get(p.parent ?? "")) {
    if (seen.has(p.id)) return true;
    seen.add(p.id);
  }
  return false;
}
export const nodeOf = computed(() => {
  const m = new Map<string, TreeNode>();
  const walk = (n: TreeNode) => { m.set(n.t.id, n); n.kids.forEach(walk); };
  forest.value.forEach(walk);
  return m;
});
export function walk(n: TreeNode, f: (t: ThreadItem) => void) { f(n.t); n.kids.forEach((k) => walk(k, f)); }
export function treeCost(n: TreeNode): number { let c = 0; walk(n, (t) => (c += t.cost ?? 0)); return c; }
/** Something in the tree works or waits on the user. */
export function hot(n: TreeNode): boolean { let h = false; walk(n, (t) => { if (t.state === "working" || t.question || t.state === "blocked") h = true; }); return h; }
