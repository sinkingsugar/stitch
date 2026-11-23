// `bun test`: the map's model: the effort roll-up, the filters, determinism (src/map/model.ts, state.ts).
import { expect, test } from "bun:test";
import type { ThreadItem } from "../src/model";
import { applyFilter, buildModel, defaultFilter, gone, selOfThread, cutoff, DAY } from "../src/map/model";
import { rowsSignature } from "../src/map/state";

const NOW = Date.parse("2026-11-12T12:00:00Z");
const iso = (daysAgo: number) => new Date(NOW - daysAgo * DAY).toISOString();
const row = (id: string, o: Partial<ThreadItem> = {}): ThreadItem => ({
  id, title: id, goal: "", node: "n", place: "p1", parent: null, state: "idle", started: iso(2), last_step: iso(1), model_calls: 3, cost: 1, last_words: null, unread: false, question: null, ...o,
} as unknown as ThreadItem);

test("a spawn tree rolls up into one effort, however deep, with its flags", () => {
  const m = buildModel([
    row("r"), row("a", { parent: "r", state: "working" }), row("b", { parent: "a", state: "blocked", question: { id: "q", command: "ls" } }), row("c", { parent: "b", unread: true }), row("solo", { place: "p2" }),
  ], NOW);
  expect(m.efforts.map((e) => e.id).sort()).toEqual(["r", "solo"]);
  const e = m.eById.get("r")!;
  expect(e.threads.length).toBe(4);
  expect(m.byId.get("c")!.rootId).toBe("r");
  expect(e.need && e.working && e.unread).toBe(true);
  expect(e.cost).toBe(4);
  expect(m.places.map((p) => [p.key, p.count, p.need, p.working, p.unread])).toEqual([["p1", 4, 1, 1, 1], ["p2", 1, 0, 0, 0]]);
  expect(m.real).toBe(5);
});

test("a child in another place stays in its parent's effort; its place keeps counting the effort once", () => {
  const m = buildModel([row("r"), row("x", { parent: "r", place: "p2" })], NOW);
  expect(m.efforts.length).toBe(1);
  expect(m.eById.get("r")!.threads.map((t) => t.placeKey).sort()).toEqual(["p1", "p2"]);
  expect(m.places.length).toBe(1);
});

test("threads whose parent is not loaded hang under a synthetic effort per place, not as roots of their own", () => {
  const m = buildModel([row("o1", { parent: "gone" }), row("o2", { parent: "gone2" }), row("o3", { parent: "gone", place: "p2" }), row("ok")], NOW);
  expect(m.orphans).toBe(3);
  expect(m.real).toBe(4);
  const s = m.byId.get("orphans:p1")!;
  expect(s.synthetic).toBe(true);
  expect(s.kids.map((k) => k.id).sort()).toEqual(["o1", "o2"]);
  expect(m.byId.get("o1")!.rootId).toBe("orphans:p1");
  expect(m.efforts.map((e) => e.id).sort()).toEqual(["ok", "orphans:p1", "orphans:p2"]);
  expect(m.places.find((p) => p.key === "p1")!.count).toBe(3); // the synthetic one is not counted
});

test("a cycle is cut at its earliest member, which becomes a root", () => {
  const m = buildModel([row("a", { parent: "c", started: iso(5) }), row("b", { parent: "a", started: iso(4) }), row("c", { parent: "b", started: iso(3) }), row("self", { parent: "self" })], NOW);
  expect(m.byId.get("a")!.cycleCut).toBe(true);
  expect(m.efforts.map((e) => e.id).sort()).toEqual(["a", "self"]);
  expect(m.eById.get("a")!.threads.length).toBe(3);
});

test("the order does not depend on the order of the rows", () => {
  const rows = [row("r"), row("a", { parent: "r", started: iso(1.5) }), row("b", { parent: "r", started: iso(1.7) }), row("z", { place: "p0" }), row("y", { place: "p2", started: iso(9) }), row("x", { place: "p2", started: iso(9) })];
  const sig = (l: ThreadItem[]) => { const m = buildModel(l, NOW); return JSON.stringify([m.places.map((p) => [p.key, p.efforts.map((e) => e.id)]), m.byId.get("r")!.kids.map((k) => k.id)]); };
  expect(sig(rows.slice().reverse())).toBe(sig(rows));
  expect(sig(rows)).toContain('["p2",["x","y"]]'); // equal starts: by id
});

test("the time filter removes efforts and places with nothing in the window; the rest stays whole", () => {
  const m = buildModel([
    row("old", { place: "old", started: iso(40), last_step: iso(39) }),
    row("mix", { started: iso(40), last_step: iso(39) }), row("mixkid", { parent: "mix", started: iso(2), last_step: iso(0.1) }),
    row("new", { place: "new", started: iso(0.3), last_step: iso(0.2) }),
  ], NOW);
  let st = applyFilter(m, { ...defaultFilter(), time: "week" }, NOW);
  expect(m.eById.get("old")!.vis).toBe(false);
  expect(m.eById.get("mix")!.vis).toBe(true); // a thread inside is active
  expect(m.pMap.get("old")!.vis).toBe(false);
  expect(st.nPlacesVis).toBe(2);
  expect(st.nEffortsVis).toBe(2);
  expect(m.byId.get("mix")!.fade).toBe(0.2); // dimmed inside the kept effort
  expect(gone(m, { kind: "place", id: "old" })).toBe(true);
  expect(gone(m, { kind: "thread", id: "mixkid" })).toBe(false);
  const sigWeek = st.layoutSig;
  st = applyFilter(m, defaultFilter(), NOW);
  expect(st.nPlacesVis).toBe(3);
  expect(st.layoutSig).not.toBe(sigWeek);
  st = applyFilter(m, { ...defaultFilter(), time: "today" }, NOW);
  expect(m.pMap.get("p1")!.vis).toBe(true);
  expect(cutoff("3d", NOW)).toBe(NOW - 3 * DAY);
});

test("search fades what does not match; needs-you dims the rest; unread marks are off when most are unread", () => {
  const m = buildModel([row("a", { title: "Fix the parser", unread: true }), row("b", { title: "Plan trip", unread: true, state: "blocked" }), row("c", { unread: true })], NOW);
  let st = applyFilter(m, { ...defaultFilter(), q: "parser" }, NOW);
  expect(st.hits).toBe(1);
  expect(m.byId.get("b")!.fade).toBe(0.1);
  st = applyFilter(m, { ...defaultFilter(), needs: true }, NOW);
  expect(m.byId.get("a")!.fade).toBe(0.12);
  expect(m.byId.get("b")!.fade).toBe(1);
  expect(st.needs).toBe(1);
  expect(st.unreadOn).toBe(false); // 100% unread: automatic is off
  expect(applyFilter(m, { ...defaultFilter(), unread: true }, NOW).unreadOn).toBe(true);
});

test("a thread's selection is its effort when it is the root", () => {
  const m = buildModel([row("r"), row("k", { parent: "r" })], NOW);
  expect(selOfThread(m.byId.get("r")!)).toEqual({ kind: "effort", id: "r" });
  expect(selOfThread(m.byId.get("k")!)).toEqual({ kind: "thread", id: "k" });
});

test("the forest's signature moves with what the map shows, and only with that", () => {
  const a = [row("a"), row("b")];
  expect(rowsSignature(a)).toBe(rowsSignature(a.map((r) => ({ ...r }))));
  expect(rowsSignature(a)).not.toBe(rowsSignature([row("a"), row("b", { state: "working" })]));
  expect(rowsSignature(a)).not.toBe(rowsSignature([row("a"), row("b", { last_step: iso(0) })]));
});
