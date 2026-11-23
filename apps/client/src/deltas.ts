// The list as the node pushes it (design §2.24, "Deltas, not re-fetches"):
// a snapshot numbered 1, then deltas numbered on, each the rows that moved
// (whole) and the threads gone. Pure functions, applied by the store.
import type { ThreadItem } from "./model";
import type { PlaceCount } from "./payload";

/** One `list` frame. */
export type ListPush = {
  v: number;
  reset?: { threads: ThreadItem[]; places?: PlaceCount[] } | null;
  upsert?: ThreadItem[];
  remove?: string[];
  places?: PlaceCount[];
};

/** What the store holds of the list. */
export type ListState = {
  v: number;
  threads: ThreadItem[];
  places: PlaceCount[];
  /** Sections read on their own (`Threads { place }`), by place. */
  placeRows: Record<string, ThreadItem[]>;
  /** The open thread's row when the window leaves it out. */
  extra: Record<string, ThreadItem | null>;
};

/** Pinned first, then the latest step, as the node orders the list. */
export const byActivity = (x: ThreadItem, y: ThreadItem) =>
  +!!y.pinned - +!!x.pinned || (y.last_step ?? "").localeCompare(x.last_step ?? "");

/** `rows` with `upsert` put in place (a new row only when it falls in the
 * window of `limit`) and `gone` taken out: a row's `earlier` (threads
 * compacted into it) are gone too. */
export function merge(rows: ThreadItem[], upsert: ThreadItem[], gone: string[], limit: number, admit = true): ThreadItem[] {
  const out = new Map(rows.map((r) => [r.id, r]));
  const drop = new Set(gone);
  for (const r of upsert) for (const e of r.earlier ?? []) drop.add(e);
  for (const id of drop) out.delete(id);
  const sorted = [...out.values()].sort(byActivity);
  const last = sorted.length >= limit ? sorted[limit - 1] : null;
  for (const r of upsert) {
    if (drop.has(r.id)) continue;
    if (out.has(r.id)) out.set(r.id, r);
    else if (admit && (!last || byActivity(r, last) < 0)) out.set(r.id, r);
  }
  return [...out.values()].sort(byActivity).slice(0, limit);
}

/** The state after `p`, or "resync" when `p` does not follow it (a frame
 * missed: ask for the list again). A reset replaces the window; the place
 * sections are the store's to read again. */
export function apply(s: ListState, p: ListPush, limit: number): ListState | "resync" {
  if (p.reset) {
    return { ...s, v: p.v, threads: [...p.reset.threads].sort(byActivity), places: p.reset.places ?? s.places };
  }
  if (p.v !== s.v + 1) return "resync";
  const up = p.upsert ?? [];
  const gone = p.remove ?? [];
  const placeRows: Record<string, ThreadItem[]> = {};
  for (const [place, rows] of Object.entries(s.placeRows)) {
    // A section holds its place's rows: one that moved elsewhere leaves it.
    const moved = up.filter((r) => r.place !== place).map((r) => r.id);
    placeRows[place] = merge(rows, up.filter((r) => r.place === place), [...gone, ...moved], Number.MAX_SAFE_INTEGER);
  }
  const extra: Record<string, ThreadItem | null> = {};
  for (const [id, row] of Object.entries(s.extra)) {
    const r = up.find((u) => u.id === id || u.earlier?.includes(id));
    extra[id] = gone.includes(id) ? null : r ?? row;
  }
  return {
    v: p.v,
    threads: merge(s.threads, up, gone, limit),
    places: p.places ?? s.places,
    placeRows,
    extra,
  };
}

/** A running call heard (`call`): the row's `in_flight`, or the rows as they were. */
export function withCall(rows: ThreadItem[], thread: string, inFlight: ThreadItem["in_flight"]): ThreadItem[] {
  const i = rows.findIndex((r) => r.id === thread);
  if (i < 0 || JSON.stringify(rows[i]!.in_flight ?? null) === JSON.stringify(inFlight ?? null)) return rows;
  const out = rows.slice();
  out[i] = { ...rows[i]!, in_flight: inFlight };
  return out;
}
