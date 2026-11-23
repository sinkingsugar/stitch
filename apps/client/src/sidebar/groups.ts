// The sidebar's sections (design §2.25, step 1): the spawn trees grouped by
// place, pinned ones first in a section of their own. Pure, so it is tested
// (test/groups.test.ts) apart from the view.
import { buildForest, walk, type TreeNode } from "../home/state";
import type { ThreadItem } from "../model";
import type { PlaceCount } from "../payload";

export interface Section {
  /** What its fold is kept under: the place, or `time:<day>` for a day's section. */
  key: string;
  /** What its header says: the place, or the day. */
  label: string;
  /** By place: the place as a row's `place` names it; by time, "". */
  place: string;
  /** A day's section (the sidebar ordered by time), not a place's. */
  time: boolean;
  /** How many listed threads the node says it has (all of them, not the window's). */
  count: number;
  /** The trees of this place that are loaded, by the latest activity in them. */
  nodes: TreeNode[];
  /** Loaded threads of the place (every node of the trees). */
  loaded: number;
  /** The latest activity among them; "" when none is loaded. */
  latest: string;
  /** Something in it waits on the user / works. */
  needs: boolean;
  working: boolean;
  /** The place's settings keep its threads out of memory. */
  memoryOff: boolean;
  /** Loaded conversations of it with activity the user has not read. */
  unread: number;
  /** Those, for "mark all read". */
  unreadRows: ThreadItem[];
}

/** How the sidebar groups the threads: by place (projects, folders, global),
 * or by the day of the latest activity in each tree, as the app's list does. */
export type Order = "place" | "time";

export interface Groups {
  pinned: TreeNode[];
  sections: Section[];
}

/** The listed threads (deduplicated by ID) and their roots: the spawn trees,
 * or with a `query`, the matches only, flat, newest first. */
function rootsOf(window: ThreadItem[], more: ThreadItem[], query: string) {
  const seen = new Set<string>();
  let list = [...window, ...more].filter((t) => !seen.has(t.id) && !!seen.add(t.id));
  const q = query.toLowerCase();
  if (q) list = list.filter((t) => (t.title + t.place + t.goal + (t.last_words ?? "")).toLowerCase().includes(q));
  // Matches are not nested: a child whose parent did not match stays visible.
  const roots = q ? list.map((t): TreeNode => ({ t, kids: [] })).sort((a, b) => (b.t.last_step ?? "").localeCompare(a.t.last_step ?? "")) : buildForest(list);
  return { list, q, roots };
}

/** A section's roll-up of the trees in it. */
function rollUp(nodes: TreeNode[], dot?: (t: ThreadItem) => string) {
  let latest = "", needs = false, working = false, loaded = 0;
  const unreadRows: ThreadItem[] = [];
  for (const n of nodes)
    walk(n, (t) => {
      loaded++;
      if ((t.last_step ?? "") > latest) latest = t.last_step ?? "";
      if (dot ? dot(t) === "waiting" : !!t.question || t.state === "blocked") needs = true;
      if (t.state === "working") working = true;
      if (t.unread) unreadRows.push(t);
    });
  return { latest, needs, working, loaded, unreadRows, unread: unreadRows.length };
}

/** `window` is the list's latest threads, `more` the threads of places read on
 * their own (deduplicated by ID), `counts` the node's count per place. With a
 * `query`, the matches only, flat, and the sections that have none go. By
 * `time`, see `groupByTime`. */
export function group(window: ThreadItem[], more: ThreadItem[], counts: PlaceCount[], query = "", dot?: (t: ThreadItem) => string, order: Order = "place", now = new Date()): Groups {
  if (order === "time") return groupByTime(window, more, query, dot, now);
  const { list, q, roots } = rootsOf(window, more, query);
  const pinned = roots.filter((n) => n.t.pinned);
  const byPlace = new Map<string, TreeNode[]>();
  for (const n of roots) if (!n.t.pinned) byPlace.set(n.t.place, [...(byPlace.get(n.t.place) ?? []), n]);
  const count = new Map(counts.map((c) => [c.place, c.count]));
  // Places the node counts, and any a loaded thread names (an old node, no counts).
  const places = new Set([...count.keys(), ...list.map((t) => t.place)]);
  const sections: Section[] = [];
  for (const place of places) {
    const nodes = byPlace.get(place) ?? [];
    // A filter hides the sections it leaves without a match (pinned ones show above).
    if (q && !nodes.length) continue;
    const r = rollUp(nodes, dot);
    let loaded = 0;
    for (const t of list) if (t.place === place) loaded++;
    // With a filter the count is the matches; else the node's, never below what is loaded.
    sections.push({ ...r, key: place, label: place, place, time: false, count: q ? loaded : Math.max(count.get(place) ?? 0, loaded), nodes, loaded, memoryOff: counts.some((c) => c.place === place && c.memory === false) });
  }
  sections.sort((a, b) => b.latest.localeCompare(a.latest) || b.count - a.count || a.place.localeCompare(b.place));
  return { pinned, sections };
}

/** The days a tree's latest activity falls in, as the app's list names them. */
const DAYS = ["Today", "Yesterday", "Earlier"] as const;
function dayOf(at: string, now: Date): (typeof DAYS)[number] {
  const d = new Date(at);
  if (!at || isNaN(+d)) return "Earlier";
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  if (d >= midnight) return "Today";
  const before = new Date(midnight);
  before.setDate(before.getDate() - 1);
  return d >= before ? "Yesterday" : "Earlier";
}

/** The sidebar by time: pinned apart, then Today, Yesterday and Earlier by the
 * latest activity in each tree (a conversation whose spawn moved today is
 * today's), newest first; only the loaded threads (the window and the places
 * read on their own). With a `query`, the matches, flat, by their own day. */
export function groupByTime(window: ThreadItem[], more: ThreadItem[], query = "", dot?: (t: ThreadItem) => string, now = new Date()): Groups {
  const { roots } = rootsOf(window, more, query);
  const pinned = roots.filter((n) => n.t.pinned);
  const byDay = new Map<string, TreeNode[]>();
  for (const n of roots) {
    if (n.t.pinned) continue;
    let latest = "";
    walk(n, (t) => { if ((t.last_step ?? "") > latest) latest = t.last_step ?? ""; });
    const day = dayOf(latest, now);
    byDay.set(day, [...(byDay.get(day) ?? []), n]);
  }
  const sections: Section[] = [];
  for (const day of DAYS) {
    const nodes = byDay.get(day);
    if (!nodes?.length) continue;
    const r = rollUp(nodes, dot);
    sections.push({ ...r, key: "time:" + day, label: day, place: "", time: true, count: r.loaded, nodes, memoryOff: false });
  }
  return { pinned, sections };
}

/** Whether a section is open when the user has not chosen: where something
 * waits or works, and the place of the latest activity. */
export function openByDefault(s: Section, first: boolean): boolean {
  return s.needs || s.working || first;
}

