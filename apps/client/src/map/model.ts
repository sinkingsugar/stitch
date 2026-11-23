// The map's data model (design §2.25, "The map"): the whole forest of rows →
// threads → efforts (a root and everything spawned under it) → places, with
// the effort roll-up, the filters and the palette. Pure: no DOM, no canvas.
// Ported from the Atlas prototype's common.js.
import type { ThreadItem } from "../model";

export const DAY = 86_400_000;

export type Kind = "need" | "work" | "idle";
export type TimeFilter = "today" | "3d" | "week" | "month" | "all";
export const TIMES: TimeFilter[] = ["today", "3d", "week", "month", "all"];

export interface MFilter {
  q: string;
  time: TimeFilter;
  needs: boolean;
  /** null: automatic (off when most threads are unread). */
  unread: boolean | null;
}
export const defaultFilter = (): MFilter => ({ q: "", time: "all", needs: false, unread: null });

export interface MThread {
  id: string;
  title: string;
  goal: string;
  node: string;
  placeKey: string;
  parent: string | null;
  state: string;
  question: { id: string; command: string } | null;
  headline: string;
  lastWords: string;
  unread: boolean;
  /** Start and last step, ms. */
  s: number;
  e: number;
  cost: number;
  calls: number;
  need: boolean;
  working: boolean;
  /** Weight for the packing. */
  w: number;
  hay: string;
  kids: MThread[];
  synthetic?: boolean;
  orphan?: boolean;
  cycleCut?: boolean;
  /** The synthetic "(parent not loaded)" thread an orphan hangs under. */
  up?: MThread;
  rootId: string;
  effort: Effort;
  fade: number;
}

export interface Effort {
  id: string;
  root: MThread;
  place: string;
  threads: MThread[];
  s: number;
  e: number;
  w: number;
  cost: number;
  need: boolean;
  working: boolean;
  unread: boolean;
  placeObj: Place;
  fade: number;
  vis: boolean;
}

export interface Place {
  key: string;
  name: string;
  efforts: Effort[];
  need: number;
  working: number;
  unread: number;
  count: number;
  cost: number;
  s: number;
  e: number;
  fade: number;
  vis: boolean;
}

export interface Model {
  threads: MThread[];
  byId: Map<string, MThread>;
  efforts: Effort[];
  eById: Map<string, Effort>;
  places: Place[];
  pMap: Map<string, Place>;
  /** Threads without the synthetic ones. */
  real: number;
  orphans: number;
}

export interface Stats {
  hits: number;
  needs: number;
  total: number;
  query: boolean;
  unread: number;
  unreadFrac: number;
  unreadOn: boolean;
  nPlacesVis: number;
  nEffortsVis: number;
  /** Changes when places or efforts appear or vanish (the layout must be packed again). */
  layoutSig: string;
}

export type Sel = { kind: "place" | "effort" | "thread"; id: string };

export const emptyModel = (): Model => ({ threads: [], byId: new Map(), efforts: [], eById: new Map(), places: [], pMap: new Map(), real: 0, orphans: 0 });

const ts = (v: string | null | undefined): number | null => {
  if (v == null || v === "") return null;
  const n = Date.parse(v);
  return Number.isNaN(n) ? null : n;
};
const byStart = (a: { s: number; id: string }, b: { s: number; id: string }) => a.s - b.s || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

export function placeName(k: string | null | undefined): string {
  if (!k || k === "global") return "global";
  const s = String(k).replace(/\/+$/, "").split("/");
  return s[s.length - 1] || k;
}

/** Rows → the model. Deterministic: efforts by start time, places by key.
 * A parent that is not among the rows (deleted, on a repository this node
 * does not hold) makes its children ORPHANS: they hang under a synthetic
 * "(parent not loaded)" thread of their place, never silently a root. A cycle
 * (should not exist) is cut at its earliest member, which becomes a root. */
export function buildModel(rows: ThreadItem[], now: number): Model {
  const byId = new Map<string, MThread>();
  const threads: MThread[] = [];
  for (const r of rows) {
    if (!r || !r.id || byId.has(r.id)) continue;
    const s = ts(r.started) ?? ts(r.last_step) ?? now;
    const t = {
      id: r.id, title: r.title || r.id, goal: r.goal ?? "", node: r.node ?? "", placeKey: r.place || "global", parent: r.parent ?? null,
      state: r.state, question: r.question ?? null, headline: r.digest?.headline ?? "", lastWords: r.last_words ?? "", unread: !!r.unread,
      s, e: Math.max(s, ts(r.last_step) ?? s), cost: Number(r.cost) || 0, calls: Number(r.model_calls) || 0,
      need: r.state === "blocked" || !!r.question, working: r.state === "working", w: 0,
      hay: [r.title, r.goal, r.digest?.headline, r.last_words].filter(Boolean).join(" \n ").toLowerCase(),
      kids: [], fade: 1,
    } as unknown as MThread;
    byId.set(t.id, t);
    threads.push(t);
  }
  // Weights: cost, else calls scaled by the median cost per call.
  const cpcs = threads.filter((t) => t.cost > 0 && t.calls > 0).map((t) => t.cost / t.calls).sort((a, b) => a - b);
  const cpc = cpcs.length ? cpcs[cpcs.length >> 1]! : 0.01;
  const w0 = threads.map((t) => (t.cost > 0 ? t.cost : t.calls > 0 ? t.calls * cpc : cpc * 3));
  const pos = w0.slice().sort((a, b) => a - b);
  const floor = (pos[Math.floor(pos.length * 0.2)] || 0.01) * 0.6;
  threads.forEach((t, i) => (t.w = Math.max(w0[i]!, floor)));

  // Cycles: walk each parent chain; a chain that meets itself is cut at its earliest member.
  const state = new Map<string, 1 | 2>();
  for (const t of threads) {
    if (state.get(t.id)) continue;
    const path: MThread[] = [];
    let c: MThread | undefined = t;
    let met: MThread | null = null; // the member of this walk that was reached again: a cycle starts there
    while (c) {
      if (state.get(c.id)) { if (state.get(c.id) === 1) met = c; break; }
      state.set(c.id, 1);
      path.push(c);
      const p: MThread | undefined = c.parent ? byId.get(c.parent) : undefined;
      if (!p || p === c) break;
      c = p;
    }
    if (met) {
      const cyc = path.slice(path.indexOf(met));
      const head = cyc.reduce((m, x) => (x.s < m.s || (x.s === m.s && x.id < m.id) ? x : m));
      head.cycleCut = true;
    }
    path.forEach((x) => state.set(x.id, 2));
  }
  const orphans = new Map<string, MThread[]>();
  let nOrphans = 0;
  for (const t of threads) {
    const p = t.parent && !t.cycleCut && t.parent !== t.id ? byId.get(t.parent) : undefined;
    if (p) p.kids.push(t);
    else if (t.parent && !t.cycleCut && t.parent !== t.id) {
      t.orphan = true;
      nOrphans++;
      let l = orphans.get(t.placeKey);
      if (!l) orphans.set(t.placeKey, (l = []));
      l.push(t);
    }
  }
  const synth: MThread[] = [];
  for (const [place, l] of [...orphans].sort((a, b) => (a[0] < b[0] ? -1 : 1))) {
    const id = "orphans:" + place;
    const r = {
      id, title: "(parent not loaded)", goal: "", node: "", placeKey: place, parent: null, state: "idle", question: null, headline: "", lastWords: "",
      synthetic: true, kids: l, s: Math.min(...l.map((x) => x.s)), e: Math.max(...l.map((x) => x.e)), cost: 0, calls: 0, w: 0,
      need: false, working: false, unread: false, hay: "", fade: 1,
    } as unknown as MThread;
    l.forEach((x) => { x.up = r; });
    byId.set(id, r);
    synth.push(r);
    threads.push(r);
  }
  if (synth.length) {
    const m = Math.min(...threads.filter((x) => !x.synthetic).map((x) => x.w));
    synth.forEach((r) => (r.w = Number.isFinite(m) && m ? m : 0.01));
  }
  // Roots: follow the (now acyclic) parent links, however deep.
  const rootOf = new Map<string, string>();
  const findRoot = (t0: MThread): string => {
    const chain: MThread[] = [];
    let r = t0;
    while (!rootOf.has(r.id)) {
      chain.push(r);
      const p: MThread | undefined | null = r.synthetic ? null : r.up || (r.parent && !r.cycleCut && r.parent !== r.id ? byId.get(r.parent) : null);
      if (!p) { rootOf.set(r.id, r.id); chain.pop(); break; }
      r = p;
    }
    const root = rootOf.get(r.id)!;
    chain.forEach((x) => rootOf.set(x.id, root));
    return root;
  };
  for (const t of threads) t.rootId = findRoot(t);
  for (const t of threads) t.kids.sort(byStart);

  const efforts: Effort[] = [];
  const eById = new Map<string, Effort>();
  for (const t of threads) {
    if (t.rootId === t.id) {
      const e = { id: t.id, root: t, place: t.placeKey, threads: [], s: t.s, e: t.e, w: 0, cost: 0, need: false, working: false, unread: false, fade: 1, vis: true } as unknown as Effort;
      eById.set(t.id, e);
      efforts.push(e);
    }
  }
  for (const t of threads) {
    const e = eById.get(t.rootId);
    if (!e) continue;
    t.effort = e;
    e.threads.push(t);
    e.s = Math.min(e.s, t.s); e.e = Math.max(e.e, t.e); e.w += t.w; e.cost += t.cost;
    e.need = e.need || t.need; e.working = e.working || t.working; e.unread = e.unread || t.unread;
  }
  const pMap = new Map<string, Place>();
  for (const e of efforts) {
    let p = pMap.get(e.place);
    if (!p) {
      p = { key: e.place, name: placeName(e.place), efforts: [], need: 0, working: 0, unread: 0, count: 0, cost: 0, s: Infinity, e: 0, fade: 1, vis: true };
      pMap.set(e.place, p);
    }
    p.efforts.push(e);
    p.count += e.threads.filter((x) => !x.synthetic).length;
    p.cost += e.cost; p.s = Math.min(p.s, e.s); p.e = Math.max(p.e, e.e);
    for (const t of e.threads) { if (t.need) p.need++; if (t.working) p.working++; if (t.unread) p.unread++; }
    e.placeObj = p;
  }
  const places = [...pMap.values()];
  places.forEach((p) => p.efforts.sort(byStart));
  places.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  // Display names that collide: add the parent folder.
  const seen = new Map<string, number>();
  places.forEach((p) => seen.set(p.name, (seen.get(p.name) || 0) + 1));
  places.forEach((p) => { if ((seen.get(p.name) ?? 0) > 1) p.name = p.key.split("/").filter(Boolean).slice(-2).join("/"); });
  return { threads, byId, efforts, eById, places, pMap, real: threads.length - synth.length, orphans: nOrphans };
}

/** The time filter's cutoff (ms): what stepped before it is out of the window. */
export function cutoff(time: TimeFilter, now: number): number {
  if (time === "today") { const d = new Date(now); d.setHours(0, 0, 0, 0); return d.getTime(); }
  if (time === "3d") return now - 3 * DAY;
  if (time === "week") return now - 7 * DAY;
  if (time === "month") return now - 30 * DAY;
  return 0;
}

/** Search tokens and toggles → a fade (0.1 … 1) per thread, effort and place;
 * the time filter REMOVES (`vis`): an effort stays whole if any of its threads
 * is active in the window, a place if any of its efforts is. */
export function applyFilter(m: Model, f: MFilter, now: number): Stats {
  const toks = f.q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const cut = cutoff(f.time, now);
  let hits = 0, needs = 0;
  for (const t of m.threads) {
    let x = 1;
    if (toks.length && !toks.every((k) => t.hay.includes(k))) x = Math.min(x, 0.1);
    else if (toks.length) hits++;
    if (cut && t.e < cut) x = Math.min(x, 0.2);
    if (f.needs && !t.need) x = Math.min(x, 0.12);
    if (t.need) needs++;
    t.fade = x;
  }
  for (const e of m.efforts) {
    e.fade = e.threads.reduce((a, t) => Math.max(a, t.fade), 0);
    e.vis = !cut || e.threads.some((t) => t.e >= cut);
  }
  let nvis = 0, nEff = 0;
  for (const p of m.places) {
    p.fade = p.efforts.reduce((a, e) => Math.max(a, e.fade), 0);
    p.vis = p.efforts.some((e) => e.vis);
    if (p.vis) nvis++;
    for (const e of p.efforts) if (e.vis && !e.root.synthetic) nEff++;
  }
  const layoutSig = m.places.map((p) => (p.vis ? p.efforts.filter((e) => e.vis).length : 0)).join(",") + "|" + m.efforts.length;
  let unread = 0;
  for (const t of m.threads) if (!t.synthetic && t.unread) unread++;
  const unreadFrac = m.real ? unread / m.real : 0;
  const unreadOn = f.unread == null ? unreadFrac <= 0.4 : f.unread;
  return { hits, needs, total: m.real, query: toks.length > 0, unread, unreadFrac, unreadOn, nPlacesVis: nvis, nEffortsVis: nEff, layoutSig };
}

export const visPlaces = (m: Model) => m.places.filter((p) => p.vis);
export const visEfforts = (p: Place) => p.efforts.filter((e) => e.vis);

export function resolve(m: Model, s: Sel | null): Place | Effort | MThread | null {
  if (!s) return null;
  if (s.kind === "place") return m.pMap.get(s.id) ?? null;
  if (s.kind === "effort") return m.eById.get(s.id) ?? null;
  return m.byId.get(s.id) ?? null;
}
/** A selection that no longer exists or is filtered out of the window. */
export function gone(m: Model, s: Sel | null): boolean {
  if (!s) return false;
  const x = resolve(m, s);
  if (!x) return true;
  if (s.kind === "place") return !(x as Place).vis;
  const e = s.kind === "effort" ? (x as Effort) : (x as MThread).effort;
  return !!e && !e.vis;
}
/** The selection a thread stands for: its effort when it is the root. */
export const selOfThread = (t: MThread): Sel => ({ kind: t.rootId === t.id ? "effort" : "thread", id: t.id });
/** Which thread a selection stands for (a place: none). */
export function threadOf(m: Model, s: Sel | null): MThread | null {
  const o = resolve(m, s);
  if (!o || !s) return null;
  return s.kind === "effort" ? (o as Effort).root : s.kind === "thread" ? (o as MThread) : null;
}
export const sameSel = (a: Sel | null, b: Sel | null) => (!a && !b) || (!!a && !!b && a.kind === b.kind && a.id === b.id);

/** The spawned tree under a root, flat, with depth (the card's rows). */
export function flatten(root: MThread): [MThread, number][] {
  const all: [MThread, number][] = [];
  (function walk(t: MThread, d: number) { all.push([t, d]); t.kids.forEach((k) => walk(k, d + 1)); })(root, 0);
  return all;
}
export const subCount = (t: MThread): number => t.kids.reduce((n, k) => n + 1 + subCount(k), 0);
export const depthOf = (t: MThread): number => (t.kids.length ? 1 + t.kids.reduce((m, k) => Math.max(m, depthOf(k)), 0) : 0);

// ---- palette: working = teal, needs you = orange, unread = hairline ring, idle = grey-blue that dims with age
export const kindOf = (o: { need: boolean; working: boolean }): Kind => (o.need ? "need" : o.working ? "work" : "idle");
export interface Style { fill: string; stroke: string; soft: string; rgb: string; text: string; dim?: number }
const clamp = (x: number, a: number, b: number) => Math.max(a, Math.min(b, x));
export const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
export const smooth = (a: number, b: number, x: number) => { const t = clamp((x - a) / (b - a), 0, 1); return t * t * (3 - 2 * t); };
const styleCache = new Map<string, Style>();
export function style(kind: Kind, ageDays: number): Style {
  const a = Math.round(clamp(ageDays / 14, 0, 1) * 20);
  const key = kind + (kind === "idle" ? a : "");
  let s = styleCache.get(key);
  if (s) return s;
  if (kind === "work") s = { fill: "rgba(22,166,150,.9)", stroke: "rgba(110,238,218,.95)", soft: "rgba(45,212,191,.12)", rgb: "45,212,191", text: "#f4fffd" };
  else if (kind === "need") s = { fill: "rgba(232,118,34,.94)", stroke: "rgba(255,196,140,.98)", soft: "rgba(251,146,60,.12)", rgb: "251,146,60", text: "#fff8f2" };
  else {
    const t = a / 20, L = lerp(50, 19, t), al = lerp(0.92, 0.5, t);
    s = { fill: `hsla(216,26%,${L}%,${al})`, stroke: `hsla(216,32%,${L + 16}%,${lerp(0.9, 0.4, t)})`, soft: `hsla(216,26%,${L + 8}%,.08)`, rgb: "111,134,168", text: t > 0.6 ? "#aab6c7" : "#eef3fa", dim: t };
  }
  styleCache.set(key, s);
  return s;
}

// ---- small formatting helpers for the card
const MON = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
export const fmtTime = (ms: number) => { const d = new Date(ms); return MON[d.getMonth()] + " " + d.getDate() + " " + String(d.getHours()).padStart(2, "0") + ":" + String(d.getMinutes()).padStart(2, "0"); };
export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, (now - ms) / 1000);
  if (s < 90) return "just now";
  if (s < 5400) return Math.round(s / 60) + " min ago";
  if (s < 129600) return Math.round(s / 3600) + " h ago";
  return Math.round(s / 86400) + " d ago";
}
export const money = (c: number) => (c >= 100 ? "$" + Math.round(c) : c >= 1 ? "$" + c.toFixed(2) : c > 0 ? "$" + c.toFixed(3) : "—");
export const stateLabel = (t: MThread) => (t.need ? (t.question ? "needs you" : "blocked") : t.state || "idle");
