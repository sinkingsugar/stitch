// `bun test`: Space's pure layout (src/map/space/layout.ts): the time scale and its levels, the radial trees (no two
// nodes too close), the rooms (no two efforts' trees overlapping), the positions on Islands' packing, the labels.
import { expect, test } from "bun:test";
import { hierarchy, pack } from "d3-hierarchy";
import { applyFilter, buildModel, defaultFilter, visEfforts, visPlaces, type Model, type MThread } from "../src/map/model";
import { HGT, S, layoutSpace, placeLabels, radialTree, rooms, spanFor, timeScale, toPack, toWorld, type LabelCand, type TNode } from "../src/map/space/layout";
import type { PackLayout } from "../src/map/space/types";
import { makeForest } from "../src/mock";

const NOW = Date.now();

/** Islands' packing (islands.ts `build`), without the canvas: places > efforts > threads, the same values and padding. */
function packOf(m: Model): PackLayout {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const mk = (t: MThread): any => {
    const kids = t.kids.map(mk);
    return kids.length ? { id: t.id, children: [{ self: true, value: Math.pow(t.w, 0.3) }, ...kids] } : { id: t.id, value: Math.pow(t.w, 0.3) };
  };
  const tree = { children: visPlaces(m).map((p) => ({ key: p.key, children: visEfforts(p).map((e) => mk(e.root)) })) };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const h = hierarchy<any>(tree).sum((d) => d.value || 0);
  pack<unknown>().size([1000, 1000]).padding((n) => (n.depth === 0 ? 14 : n.depth === 1 ? 3 : n.depth === 2 ? 1.8 : 1.2))(h);
  const out: PackLayout = { size: 1000, places: new Map(), efforts: new Map() };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  for (const p of (h.children ?? []) as any[]) {
    out.places.set(p.data.key, { x: p.x, y: p.y, r: p.r });
    for (const e of p.children ?? []) out.efforts.set(e.data.id, { x: e.x, y: e.y, r: e.r });
  }
  return out;
}
const seeded = (seed: number) => () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);
function randomTree(n: number, rnd: () => number): TNode {
  const all: TNode[] = [{ id: "n0", kids: [] }];
  for (let i = 1; i < n; i++) {
    const p = all[Math.floor(rnd() * rnd() * all.length)]!;
    const c = { id: "n" + i, kids: [] };
    p.kids.push(c);
    all.push(c);
  }
  return all[0]!;
}
const minDist = (pts: { x: number; z: number }[]) => {
  let m = Infinity;
  for (let i = 0; i < pts.length; i++) for (let j = i + 1; j < pts.length; j++) m = Math.min(m, Math.hypot(pts[i]!.x - pts[j]!.x, pts[i]!.z - pts[j]!.z));
  return m;
};

test("height is recency on a log scale: now at the top, the span's end on the ground, levels inside the span", () => {
  const s = timeScale(40);
  expect(s.y(0)).toBe(HGT);
  expect(s.y(40)).toBeCloseTo(0, 6);
  expect(s.y(400)).toBe(0);
  let last = Infinity;
  for (const d of [0, 0.1, 0.5, 1, 3, 7, 14, 30, 40]) { expect(s.y(d)).toBeLessThan(last); last = s.y(d); }
  // Log: the first day takes more height than the next three.
  expect(s.y(0) - s.y(1)).toBeGreaterThan(s.y(1) - s.y(4));
  expect(s.levels.map((l) => l.label)).toEqual(["today", "yesterday", "3 days", "a week", "a month"]);
  expect(s.levels.every((l, i, a) => i === 0 || l.y < a[i - 1]!.y)).toBe(true);
  expect(timeScale(spanFor(0.5)).levels.map((l) => l.label)).toEqual(["today", "yesterday"]);
  expect(spanFor(0)).toBe(1.5);
  expect(spanFor(1000)).toBe(120);
});

test("a radial tree keeps every pair of nodes at least the gap apart, whatever its shape", () => {
  const rnd = seeded(7);
  for (const n of [1, 2, 3, 8, 30, 61, 150]) {
    for (let k = 0; k < 4; k++) {
      const t = randomTree(n, rnd);
      const { pos, ext } = radialTree(t, 1, 1.15, rnd() * 6);
      expect(pos.size).toBe(n);
      expect(pos.get("n0")).toEqual({ x: 0, z: 0, depth: 0 });
      if (n > 1) expect(minDist([...pos.values()])).toBeGreaterThan(1 - 1e-9);
      for (const p of pos.values()) expect(Math.hypot(p.x, p.z)).toBeLessThanOrEqual(ext + 1e-9);
    }
  }
  // A star, and a chain.
  const star: TNode = { id: "r", kids: Array.from({ length: 40 }, (_, i) => ({ id: "k" + i, kids: [] })) };
  expect(minDist([...radialTree(star).pos.values()])).toBeGreaterThan(1 - 1e-9);
  let chain: TNode = { id: "c9", kids: [] };
  for (let i = 8; i >= 0; i--) chain = { id: "c" + i, kids: [chain] };
  expect(radialTree(chain).ext).toBeCloseTo(9, 6);
});

test("a radial tree is deterministic and does not loop on a cycle", () => {
  const rnd = seeded(3);
  const t = randomTree(40, rnd);
  expect([...radialTree(t, 1, 1, 0.3).pos.entries()]).toEqual([...radialTree(t, 1, 1, 0.3).pos.entries()]);
  const a: TNode = { id: "a", kids: [] }, b: TNode = { id: "b", kids: [a] };
  a.kids.push(b);
  expect(radialTree(a).pos.size).toBe(2);
});

test("rooms never overlap: for any pair, the two rooms fit in their distance", () => {
  const rnd = seeded(11);
  const cs = Array.from({ length: 120 }, () => ({ x: rnd() * 1000, y: rnd() * 1000, r: 2 + rnd() * 20 }));
  // Make them disjoint, as a packing is.
  const ok = cs.filter((c, i) => cs.every((d, j) => j >= i || Math.hypot(c.x - d.x, c.y - d.y) > c.r + d.r));
  const r = rooms(ok);
  for (let i = 0; i < ok.length; i++) {
    expect(r[i]!).toBeGreaterThanOrEqual(ok[i]!.r);
    for (let j = i + 1; j < ok.length; j++) expect(r[i]! + r[j]!).toBeLessThanOrEqual(Math.hypot(ok[i]!.x - ok[j]!.x, ok[i]!.y - ok[j]!.y) + 1e-9);
  }
  expect(rooms([{ x: 0, y: 0, r: 5 }])).toEqual([10]);
});

for (const big of [false, true]) {
  test(`the mock forest${big ? " (big)" : ""}: Islands' positions, trees inside their rooms, nothing too close`, () => {
    const m = buildModel(makeForest(big), NOW);
    applyFilter(m, defaultFilter(), NOW);
    const p = packOf(m);
    const L = layoutSpace(m, p, NOW);
    if (big) { expect(m.real).toBeGreaterThan(1300); expect(m.efforts.length).toBeGreaterThan(400); }
    expect(L.items.length).toBe(m.threads.length);
    expect(L.efforts.length).toBe(m.efforts.length);
    // Each effort's root stands exactly where Islands packs the effort; each place's ring is its circle.
    for (const r of L.efforts) {
      const c = p.efforts.get(r.e.id)!, w = toWorld(p, c.x, c.y), it = L.items[r.rootIdx]!;
      expect(it.x).toBeCloseTo(w.x, 6); expect(it.z).toBeCloseTo(w.z, 6);
      const back = toPack(p, it.x, it.z);
      expect(back.x).toBeCloseTo(c.x, 6); expect(back.y).toBeCloseTo(c.y, 6);
    }
    for (const pl of L.places) expect(pl.r).toBeCloseTo(p.places.get(pl.p.key)!.r * S, 6);
    // Trees stay in their room, so two efforts' trees never interpenetrate.
    const room = rooms(L.efforts.map((r) => p.efforts.get(r.e.id)!));
    L.efforts.forEach((r, i) => {
      for (const k of r.items) expect(Math.hypot(L.items[k]!.x - r.cx, L.items[k]!.z - r.cz)).toBeLessThanOrEqual(room[i]! * S + 1e-6);
    });
    // Inside a tree no two threads closer than their size; children of a root never under it.
    for (const r of L.efforts) {
      const pts = r.items.map((k) => L.items[k]!);
      for (let a = 0; a < pts.length; a++) for (let b = a + 1; b < pts.length; b++) {
        const d = Math.hypot(pts[a]!.x - pts[b]!.x, pts[a]!.z - pts[b]!.z);
        expect(d).toBeGreaterThanOrEqual((pts[a]!.size + pts[b]!.size) / 2 - 1e-6);
      }
    }
    // Every spawned thread is joined to its parent in the same effort.
    for (const it of L.items) if (!it.root && !it.t.synthetic) expect(it.parent).toBeGreaterThanOrEqual(0);
    // Lone conversations are drawn smaller than roots with a tree.
    const lone = L.items.filter((i) => i.root && i.t.kids.length === 0), roots = L.items.filter((i) => i.root && i.t.kids.length > 2);
    expect(Math.max(...lone.map((i) => i.size))).toBeLessThan(Math.min(...roots.map((i) => i.size)));
  });
}

test("the time filter re-lays out: only what is in the window, the column rescaled", () => {
  const m = buildModel(makeForest(), NOW);
  applyFilter(m, { ...defaultFilter(), time: "week" }, NOW);
  const L = layoutSpace(m, packOf(m), NOW);
  const shown = new Set(L.efforts.map((r) => r.e.id));
  expect(shown.size).toBe(m.efforts.filter((e) => e.vis).length);
  expect(shown.size).toBeLessThan(m.efforts.length);
  expect(L.scale.span).toBeLessThanOrEqual(120);
  expect(L.scale.levels.at(-1)!.label).not.toBe("3 months");
});

test("labels never overlap, keep what was shown, put the forced ones first, and stay in the box", () => {
  const box = { x: 0, y: 0, w: 400, h: 300 };
  const c = (key: string, x: number, y: number, prio: number, o: Partial<LabelCand> = {}): LabelCand => ({ key, x, y, w: 80, h: 14, prio, shown: false, force: false, on: false, ...o });
  const a = c("a", 100, 100, 1), b = c("b", 120, 105, 1.4), d = c("d", 300, 100, 0.5), out = c("out", 395, 100, 9), f = c("f", 110, 100, 0, { force: true });
  let all = [a, b, d, out];
  expect(placeLabels(all, 10, box)).toBe(2);
  expect([a.on, b.on, d.on, out.on]).toEqual([false, true, true, false]);
  // Shown last time: kept against a slightly better newcomer (no flicker).
  a.shown = true; b.shown = false;
  placeLabels(all, 10, box);
  expect([a.on, b.on]).toEqual([true, false]);
  // At most `max`.
  placeLabels(all, 1, box);
  expect(all.filter((x) => x.on).length).toBe(1);
  // A forced one wins its room.
  all = [a, b, f];
  placeLabels(all, 10, box);
  expect(f.on).toBe(true);
  expect(a.on || b.on).toBe(false);
  // None of the placed ones overlap (random crowd).
  const rnd = seeded(5);
  const crowd = Array.from({ length: 200 }, (_, i) => c("c" + i, rnd() * 400, rnd() * 300, rnd()));
  placeLabels(crowd, 50, box, 4);
  const on = crowd.filter((x) => x.on);
  expect(on.length).toBeGreaterThan(5);
  for (let i = 0; i < on.length; i++) for (let j = i + 1; j < on.length; j++) {
    const p = on[i]!, q = on[j]!;
    const sep = Math.abs(p.x - q.x) >= (p.w + q.w) / 2 + 4 || Math.abs(p.y - q.y) >= (p.h + q.h) / 2 + 4 - 1e-9;
    expect(sep).toBe(true);
  }
});
