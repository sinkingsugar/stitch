// Threads by the conversation they came from (the dashboard's "Threads"): each
// thread goes under the conversation its spawn chain starts from, the first
// parent not in the list (a chat, or an older thread), and the threads it
// spawned hang under it.
import type { ThreadItem } from "../model";
import { dotOf, lastOf, titleOf, type Dot } from "../home/util";

export interface Stack { t: ThreadItem; kids: Stack[] }
export interface Lane {
  key: string;
  title: string;
  /** The conversation the lane is under, when it is not a card of its own. */
  conversation: string | null;
  threads: ThreadItem[];
  roots: Stack[];
  latest: number;
  counts: Record<Dot, number>;
  live: boolean;
  places: string[];
  nodes: string[];
}

const RANK: Record<Dot, number> = { working: 0, waiting: 1, failed: 2, done: 3 };
const at = (t: ThreadItem) => Date.parse(lastOf(t)) || 0;

export function lanesOf(threads: ThreadItem[]): Lane[] {
  const byId = new Map(threads.map((t) => [t.id, t]));
  const rootOf = (t: ThreadItem) => {
    const seen = new Set<string>([t.id]);
    while (t.parent && byId.has(t.parent) && !seen.has(t.parent)) {
      seen.add(t.parent);
      t = byId.get(t.parent)!;
    }
    return t;
  };
  const lanes = new Map<string, Lane>();
  for (const t of threads) {
    const root = rootOf(t);
    const key = root.parent || root.id;
    let l = lanes.get(key);
    if (!l) {
      l = {
        key, title: root.parent ? root.parent_title || "A conversation" : titleOf(root), conversation: root.parent,
        threads: [], roots: [], latest: 0, counts: { working: 0, waiting: 0, failed: 0, done: 0 }, live: false, places: [], nodes: [],
      };
      lanes.set(key, l);
    }
    l.threads.push(t);
  }
  const tally = (ts: ThreadItem[], f: (t: ThreadItem) => string) => {
    const m = new Map<string, number>();
    ts.forEach((t) => m.set(f(t), (m.get(f(t)) ?? 0) + 1));
    return [...m.entries()].sort((a, b) => b[1] - a[1]).map(([v]) => v).filter(Boolean);
  };
  for (const l of lanes.values()) {
    l.latest = Math.max(...l.threads.map(at));
    l.threads.forEach((t) => l.counts[dotOf(t)]++);
    l.live = l.counts.working + l.counts.waiting > 0;
    l.places = tally(l.threads, (t) => t.place);
    l.nodes = tally(l.threads, (t) => t.node);
    l.roots = stacks(l.threads);
  }
  const rank = (l: Lane) => (l.counts.waiting ? 0 : l.counts.working ? 1 : 2);
  return [...lanes.values()].sort((a, b) => rank(a) - rank(b) || b.latest - a.latest);
}

/** A lane's cards: working first, then waiting, failed, done, the latest first;
 * each with the threads it spawned under it. */
function stacks(ts: ThreadItem[]): Stack[] {
  const inLane = new Set(ts.map((t) => t.id));
  const order = (a: ThreadItem, b: ThreadItem) => RANK[dotOf(a)] - RANK[dotOf(b)] || at(b) - at(a);
  const kids = new Map<string, ThreadItem[]>();
  const roots: ThreadItem[] = [];
  for (const t of [...ts].sort(order)) {
    if (t.parent && t.parent !== t.id && inLane.has(t.parent)) {
      if (!kids.has(t.parent)) kids.set(t.parent, []);
      kids.get(t.parent)!.push(t);
    } else roots.push(t);
  }
  const seen = new Set<string>();
  const stack = (t: ThreadItem): Stack | null => {
    if (seen.has(t.id)) return null;
    seen.add(t.id);
    return { t, kids: (kids.get(t.id) ?? []).map(stack).filter((x): x is Stack => !!x) };
  };
  // Roots first; then anything a cycle left out.
  return [...roots, ...ts].map(stack).filter((x): x is Stack => !!x);
}
