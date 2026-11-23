// Space's layout (design §2.25, "The map": Space): pure, no three.js, no DOM, tested.
//   - the ground plane is Islands' packing: each place and effort stands exactly where Islands draws it (pack units ×
//     `S`, centred), so 2D and 3D are one geography;
//   - height is recency on a log-time scale (`timeScale`), with labelled levels;
//   - each effort is its root with the spawned threads around it on a radial tree (`radialTree`) whose nodes never come
//     closer than a gap, scaled into the room the packing leaves the effort (`rooms`), so trees never interpenetrate;
//   - labels are chosen and placed in screen space without overlapping (`placeLabels`).
import { DAY, visEfforts, visPlaces, type Effort, type Model, type MThread, type Place } from "../model";
import type { PackLayout } from "./types";

/** Height of the time column, world units. */
export const HGT = 330;
/** Pack units → world units. */
export const S = 1.3;
/** The log scale's knee, days: finer resolution under it. */
const KNEE = 0.25;

export interface Level { days: number; label: string; y: number }
export interface TimeScale { span: number; y(ageDays: number): number; levels: Level[] }

const LEVELS: [number, string][] = [[0, "today"], [1, "yesterday"], [3, "3 days"], [7, "a week"], [30, "a month"], [91, "3 months"]];

/** The span (days) the column covers: a little more than the oldest thing shown, 1.5 days to 4 months. */
export const spanFor = (maxAgeDays: number) => Math.min(120, Math.max(1.5, maxAgeDays * 1.08));

/** Height for an age: `HGT` now, 0 at the span's end, log in between. Levels inside the span, top first. */
export function timeScale(span: number): TimeScale {
  const den = Math.log1p(span / KNEE);
  const y = (age: number) => HGT * (1 - Math.min(1, Math.log1p(Math.max(0, age) / KNEE) / den));
  const levels = LEVELS.filter(([d]) => d <= span).map(([days, label]) => ({ days, label, y: y(days) }));
  return { span, y, levels };
}

// ---------------------------------------------------------------- radial tree
export interface TNode { id: string; kids: TNode[] }
export interface Placed { x: number; z: number; depth: number }

/** A radial tree in unit space: the root at the origin, each depth on a ring, each subtree in a wedge proportional to
 * its leaves (so edges do not cross). Each ring's radius is the larger of the ring inside it plus `step` and what its
 * closest pair of neighbours needs to be `gap` apart (a chord), so any two nodes are at least min(gap, step) apart.
 * `offset`: the angle the first child starts at. Deterministic: children in the order given. */
export function radialTree(root: TNode, gap = 1, step = 1, offset = 0): { pos: Map<string, Placed>; ext: number } {
  const leaves = new Map<string, number>();
  const count = (t: TNode): number => {
    if (leaves.has(t.id)) return leaves.get(t.id)!; // met again: a cycle (the model cuts them), never loop
    leaves.set(t.id, 1);
    let n = 0;
    for (const k of t.kids) n += count(k);
    n = Math.max(1, n);
    leaves.set(t.id, n);
    return n;
  };
  count(root);
  const ang = new Map<string, number>();
  const byDepth: { id: string; a: number }[][] = [];
  const seen = new Set<string>([root.id]);
  const walk = (t: TNode, a0: number, a1: number, depth: number) => {
    const total = t.kids.reduce((s, k) => s + leaves.get(k.id)!, 0);
    let a = a0;
    for (const k of t.kids) {
      if (seen.has(k.id)) continue; // a cycle (the model cuts them): never loop
      seen.add(k.id);
      const w = (a1 - a0) * leaves.get(k.id)! / total;
      const mid = a + w / 2;
      ang.set(k.id, mid);
      (byDepth[depth + 1] ??= []).push({ id: k.id, a: mid });
      walk(k, a, a + w, depth + 1);
      a += w;
    }
  };
  walk(root, offset, offset + Math.PI * 2, 0);
  const radius: number[] = [0];
  for (let d = 1; d < byDepth.length; d++) {
    const ring = (byDepth[d] ?? []).map((q) => q.a).sort((a, b) => a - b);
    let g = Math.PI * 2;
    for (let i = 0; i < ring.length; i++) {
      const next = i + 1 < ring.length ? ring[i + 1]! : ring[0]! + Math.PI * 2;
      if (ring.length > 1) g = Math.min(g, next - ring[i]!);
    }
    const chord = g >= Math.PI ? gap / 2 : gap / (2 * Math.sin(g / 2));
    radius[d] = Math.max(radius[d - 1]! + step, chord);
  }
  const pos = new Map<string, Placed>([[root.id, { x: 0, z: 0, depth: 0 }]]);
  for (let d = 1; d < byDepth.length; d++) for (const q of byDepth[d] ?? []) pos.set(q.id, { x: Math.cos(q.a) * radius[d]!, z: Math.sin(q.a) * radius[d]!, depth: d });
  return { pos, ext: radius[radius.length - 1] ?? 0 };
}

/** The room of each circle: its radius plus half the clearance to its nearest neighbour. Discs of these radii never
 * overlap (for any pair, room_i + room_j <= their distance). */
export function rooms(cs: { x: number; y: number; r: number }[]): number[] {
  return cs.map((a, i) => {
    let clear = Infinity;
    for (let j = 0; j < cs.length; j++) {
      if (j === i) continue;
      const b = cs[j]!;
      clear = Math.min(clear, Math.hypot(a.x - b.x, a.y - b.y) - a.r - b.r);
    }
    return a.r + (Number.isFinite(clear) ? Math.max(0, clear) / 2 : a.r);
  });
}

// ---------------------------------------------------------------- the scene's layout
/** How a node is drawn: a spawned thread, a root with a tree, a lone conversation, working, needing you. */
export const NK = { Child: 0, Root: 1, Lone: 2, Work: 3, Need: 4 } as const;
export type NK = (typeof NK)[keyof typeof NK];

export interface SItem {
  t: MThread;
  x: number; y: number; z: number;
  /** Index of its effort in `efforts`. */
  eff: number;
  /** Index of its parent in `items` (-1: the root). */
  parent: number;
  root: boolean;
  /** World diameter. */
  size: number;
  kind: NK;
}
export interface SEffort {
  e: Effort;
  cx: number; cz: number;
  /** Mean and top height of its threads. */
  cy: number; top: number;
  /** Horizontal extent of its tree from its centre (world). */
  ext: number;
  rootIdx: number;
  items: number[];
  /** Threads, not counting a synthetic root. */
  n: number;
  /** How much it deserves a label in the overview. */
  score: number;
}
export interface SPlace { p: Place; cx: number; cz: number; r: number; top: number }
export interface SpaceLayout {
  items: SItem[];
  efforts: SEffort[];
  places: SPlace[];
  byId: Map<string, number>;
  eIdx: Map<string, number>;
  scale: TimeScale;
  /** World radius of the whole map. */
  radius: number;
}

/** Spacing (world units) of a lone root's "tree", and the most a tree's nodes are spread apart. */
const UNIT = 7;
const UNIT_MAX = 20;
const hash01 = (s: string) => { let h = 2166136261; for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619); return (h >>> 0) / 4294967296; };

/** Pack units → world (x, z). */
export const toWorld = (pack: PackLayout, x: number, y: number) => ({ x: (x - pack.size / 2) * S, z: (y - pack.size / 2) * S });
/** World (x, z) → pack units. */
export const toPack = (pack: { size: number }, x: number, z: number) => ({ x: x / S + pack.size / 2, y: z / S + pack.size / 2 });

export function layoutSpace(m: Model, pack: PackLayout | null, now: number): SpaceLayout {
  const out: SpaceLayout = { items: [], efforts: [], places: [], byId: new Map(), eIdx: new Map(), scale: timeScale(1.5), radius: 1 };
  if (!pack) return out;
  out.radius = (pack.size / 2) * S;
  const vp = visPlaces(m);
  let maxAge = 0;
  const effs: { e: Effort; c: { x: number; y: number; r: number } }[] = [];
  for (const p of vp) for (const e of visEfforts(p)) {
    const c = pack.efforts.get(e.id);
    if (!c) continue;
    effs.push({ e, c });
    for (const t of e.threads) if (!t.synthetic) maxAge = Math.max(maxAge, (now - t.e) / DAY);
  }
  const scale = (out.scale = timeScale(spanFor(maxAge)));
  const room = rooms(effs.map((q) => q.c));
  effs.forEach(({ e, c }, ei) => {
    const w = toWorld(pack, c.x, c.y);
    const real = e.threads.filter((t) => !t.synthetic).length;
    const tree = radialTree(e.root, 1, 1.15, hash01(e.id) * Math.PI * 2);
    const roomW = room[ei]! * S * 0.94;
    // The tree spread over most of its room (so its edges read from afar), never past it, never looser than UNIT_MAX.
    const unit = tree.ext > 0 ? Math.min(UNIT_MAX, (roomW * 0.82) / tree.ext) : UNIT;
    const rec: SEffort = { e, cx: w.x, cz: w.z, cy: 0, top: 0, ext: tree.ext * unit, rootIdx: -1, items: [], n: real, score: 0 };
    const lone = e.root.kids.length === 0;
    for (const t of e.threads) {
      const q = tree.pos.get(t.id);
      if (!q) continue;
      const isRoot = t.id === e.root.id;
      const kind: NK = t.need ? NK.Need : t.working ? NK.Work : isRoot ? (lone ? NK.Lone : NK.Root) : NK.Child;
      // Children never wider than their spacing; a root with a tree grows with it, never into its first ring
      // (at 1.15 units at least).
      const child = Math.min(4.2, unit * 0.62);
      const size = isRoot ? (lone ? 3.6 : Math.min(5 + 1.6 * Math.log2(real + 1), Math.max(child * 1.3, 2.3 * unit - child))) : child;
      const i = out.items.length;
      out.byId.set(t.id, i);
      rec.items.push(i);
      if (isRoot) rec.rootIdx = i;
      out.items.push({ t, x: w.x + q.x * unit, y: scale.y((now - t.e) / DAY), z: w.z + q.z * unit, eff: out.efforts.length, parent: -1, root: isRoot, size, kind });
    }
    let sy = 0;
    for (const i of rec.items) { const y = out.items[i]!.y; sy += y; rec.top = Math.max(rec.top, y); }
    rec.cy = rec.items.length ? sy / rec.items.length : 0;
    const rec01 = 1 - Math.min(1, Math.log1p(Math.max(0, now - e.e) / DAY / KNEE) / Math.log1p(scale.span / KNEE));
    rec.score = Math.log2(real + 1) / 6 + 0.8 * rec01 + (e.need ? 1 : 0) + (e.working ? 0.6 : 0) - (e.root.synthetic ? 0.5 : 0);
    out.eIdx.set(e.id, out.efforts.length);
    out.efforts.push(rec);
  });
  for (const it of out.items) {
    if (it.root) continue;
    const t = it.t;
    const pid = t.up?.id ?? t.parent;
    const p = pid ? out.byId.get(pid) : undefined;
    if (p !== undefined && out.items[p]!.eff === it.eff) it.parent = p;
  }
  for (const p of vp) {
    const c = pack.places.get(p.key);
    if (!c) continue;
    const w = toWorld(pack, c.x, c.y);
    let top = 0;
    for (const e of visEfforts(p)) { const i = out.eIdx.get(e.id); if (i !== undefined) top = Math.max(top, out.efforts[i]!.top); }
    out.places.push({ p, cx: w.x, cz: w.z, r: c.r * S, top });
  }
  return out;
}

// ---------------------------------------------------------------- labels
export interface LabelCand {
  key: string;
  /** Anchor, screen px: the label's bottom centre. */
  x: number; y: number;
  w: number; h: number;
  prio: number;
  /** Shown in the previous placement: it gets a bonus, so labels do not swap back and forth while the camera moves. */
  shown: boolean;
  /** Always shown (the selection, the hovered thread); still claims its room. */
  force: boolean;
  /** Out: placed. */
  on: boolean;
}
export interface Rect { x: number; y: number; w: number; h: number }
export const rectOf = (c: LabelCand): Rect => ({ x: c.x - c.w / 2, y: c.y - c.h, w: c.w, h: c.h });
const hits = (a: LabelCand, b: LabelCand, pad: number) =>
  a.x - a.w / 2 - pad < b.x + b.w / 2 && a.x + a.w / 2 + pad > b.x - b.w / 2 && a.y - a.h - pad < b.y && a.y + pad > b.y - b.h;

let keepBonus = 0.6;
const byPrio = (a: LabelCand, b: LabelCand) =>
  (b.force ? 1 : 0) - (a.force ? 1 : 0) || b.prio + (b.shown ? keepBonus : 0) - (a.prio + (a.shown ? keepBonus : 0)) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
const placedScratch: LabelCand[] = [];

/** Greedy placement by priority (sorts `cands` in place, sets each one's `on`): no two placed labels closer than
 * `pad` px, all inside the box, at most `max` that are not forced. Allocates nothing (it runs every frame the camera
 * moves). Returns how many were placed. */
export function placeLabels(cands: LabelCand[], max: number, box: Rect, pad = 5, keep = 0.6): number {
  keepBonus = keep;
  cands.sort(byPrio);
  placedScratch.length = 0;
  let n = 0;
  for (const c of cands) {
    c.on = false;
    if (!c.force && n >= max) continue;
    const x0 = c.x - c.w / 2, y0 = c.y - c.h;
    const inside = x0 >= box.x && y0 >= box.y && x0 + c.w <= box.x + box.w && c.y <= box.y + box.h;
    if (!c.force && !inside) continue;
    let clash = false;
    if (!c.force) for (const q of placedScratch) if (hits(c, q, pad)) { clash = true; break; }
    if (clash) continue;
    c.on = true;
    placedScratch.push(c);
    if (!c.force) n++;
  }
  const placed = placedScratch.length;
  placedScratch.length = 0;
  return placed;
}
