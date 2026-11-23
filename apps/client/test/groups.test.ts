// The sidebar's sections by place (src/sidebar/groups.ts, design §2.25).
import { expect, test } from "bun:test";
import { group, openByDefault } from "../src/sidebar/groups";
import type { ThreadItem } from "../src/model";

const ago = (s: number) => new Date(Date.now() - s * 1000).toISOString();
function t(id: string, place: string, over: Partial<ThreadItem> = {}): ThreadItem {
  return {
    id, title: id, goal: "Work on " + id, node: "gamer", started_on: "gamer", place, parent: null, parent_title: null,
    detached: true, read_only: false, pinned: false, app: null, state: "idle", outcome: "done", said: "idle", question: null,
    started: ago(3600), last_step: ago(600), model_calls: 1, cost: null, last_words: null, in_flight: null, ...over,
  } as ThreadItem;
}

test("threads are grouped by place, pinned apart, sections by latest activity", () => {
  const g = group(
    [t("a", "stitch", { last_step: ago(100) }), t("b", "otter", { last_step: ago(10) }), t("c", "stitch", { last_step: ago(500) }), t("p", "otter", { pinned: true })],
    [],
    [{ place: "stitch", count: 2 }, { place: "otter", count: 2 }],
  );
  expect(g.pinned.map((n) => n.t.id)).toEqual(["p"]);
  expect(g.sections.map((s) => s.place)).toEqual(["otter", "stitch"]);
  expect(g.sections[1]!.nodes.map((n) => n.t.id)).toEqual(["a", "c"]);
  expect(g.sections[0]!.nodes.map((n) => n.t.id)).toEqual(["b"]);
});

test("a place the window does not cover is still a section, with the node's count, after the loaded ones", () => {
  const g = group([t("a", "stitch")], [], [{ place: "stitch", count: 1 }, { place: "small", count: 7 }, { place: "tiny", count: 2 }]);
  expect(g.sections.map((s) => [s.place, s.count, s.loaded])).toEqual([["stitch", 1, 1], ["small", 7, 0], ["tiny", 2, 0]]);
  // Its threads, read on their own, fill it (and a thread in both lists is one).
  const h = group([t("a", "stitch")], [t("old", "small", { last_step: ago(99999) }), t("a", "stitch")], [{ place: "stitch", count: 1 }, { place: "small", count: 7 }]);
  const small = h.sections.find((s) => s.place === "small")!;
  expect([small.loaded, small.count, small.nodes.length]).toEqual([1, 7, 1]);
  expect(h.sections.find((s) => s.place === "stitch")!.nodes).toHaveLength(1);
});

test("a section says whether something in it waits on the user or works, spawned threads included", () => {
  const g = group(
    [t("root", "stitch"), t("kid", "stitch", { parent: "root", state: "blocked", question: { id: "q", command: "git push" } }), t("w", "otter", { state: "working" }), t("i", "global")],
    [],
    [],
  );
  const by = Object.fromEntries(g.sections.map((s) => [s.place, s]));
  expect([by.stitch!.needs, by.stitch!.working]).toEqual([true, false]);
  expect([by.otter!.needs, by.otter!.working]).toEqual([false, true]);
  expect([by.global!.needs, by.global!.working]).toEqual([false, false]);
  expect(by.stitch!.nodes[0]!.kids.map((n) => n.t.id)).toEqual(["kid"]);
  expect(openByDefault(by.stitch!, false)).toBe(true);
  expect(openByDefault(by.global!, false)).toBe(false);
  expect(openByDefault(by.global!, true)).toBe(true);
});

test("the filter matches across places, flat, and hides sections with none", () => {
  const list = [t("alpha", "stitch"), t("beta", "otter"), t("alpine", "otter", { parent: "beta" }), t("gamma", "global", { pinned: true, title: "alp pinned" })];
  const g = group(list, [], [{ place: "stitch", count: 1 }, { place: "otter", count: 2 }, { place: "empty", count: 9 }], "alp");
  expect(g.sections.map((s) => s.place).sort()).toEqual(["otter", "stitch"]);
  // Flat: the child of a non-matching parent stays visible, un-nested.
  expect(g.sections.find((s) => s.place === "otter")!.nodes.map((n) => n.t.id)).toEqual(["alpine"]);
  expect(g.pinned.map((n) => n.t.id)).toEqual(["gamma"]);
});

test("a place whose settings keep it out of memory says so", () => {
  const g = group([t("a", "stitch"), t("b", "private")], [], [{ place: "stitch", count: 1 }, { place: "private", count: 1, memory: false }]);
  expect(Object.fromEntries(g.sections.map((s) => [s.place, s.memoryOff]))).toEqual({ stitch: false, private: true });
});

test("a section counts its unread conversations, spawned ones too, and lists them for mark-all-read", () => {
  const g = group(
    [t("a", "stitch", { unread: true }), t("b", "stitch"), t("k", "stitch", { parent: "b", unread: true }), t("o", "otter")],
    [],
    [],
  );
  const by = Object.fromEntries(g.sections.map((s) => [s.place, s]));
  expect(by.stitch!.unread).toBe(2);
  expect(by.stitch!.unreadRows.map((x) => x.id).sort()).toEqual(["a", "k"]);
  expect(by.otter!.unread).toBe(0);
});

test("by time: pinned apart, then Today, Yesterday, Earlier by each tree's latest activity", () => {
  const now = new Date(2026, 10, 17, 12, 0);
  const at = (d: number, h: number) => new Date(2026, 10, 17 - d, h).toISOString();
  const g = group(
    [
      t("old", "stitch", { last_step: at(5, 9) }),
      t("kid", "otter", { parent: "old2", last_step: at(0, 10) }),
      t("old2", "otter", { last_step: at(3, 9) }),
      t("y", "global", { last_step: at(1, 23) }),
      t("today", "stitch", { last_step: at(0, 1), unread: true }),
      t("p", "otter", { pinned: true, last_step: at(9, 9) }),
    ],
    [],
    [{ place: "stitch", count: 40 }],
    "",
    undefined,
    "time",
    now,
  );
  expect(g.pinned.map((n) => n.t.id)).toEqual(["p"]);
  // A conversation whose spawn moved today is today's; no place sections.
  expect(g.sections.map((s) => [s.key, s.label, s.time, s.count])).toEqual([["time:Today", "Today", true, 3], ["time:Yesterday", "Yesterday", true, 1], ["time:Earlier", "Earlier", true, 1]]);
  expect(g.sections[0]!.nodes.map((n) => n.t.id)).toEqual(["old2", "today"]);
  expect(g.sections[0]!.unread).toBe(1);
  // A filter keeps only the days with a match.
  const f = group([t("a", "x", { last_step: at(0, 9) }), t("b", "x", { last_step: at(4, 9), title: "needle" })], [], [], "needle", undefined, "time", now);
  expect(f.sections.map((s) => s.label)).toEqual(["Earlier"]);
});
