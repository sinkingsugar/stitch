// The dashboard's Threads lanes (src/dashboard/lanes.ts): a lane per
// conversation, spawn trees under it, working and waiting lanes first.
import { expect, test } from "bun:test";
import { lanesOf } from "../src/dashboard/lanes";
import type { ThreadItem } from "../src/model";

const ago = (s: number) => new Date(Date.now() - s * 1000).toISOString();
function t(id: string, over: Partial<ThreadItem> = {}): ThreadItem {
  return {
    id, title: id, goal: "Work on " + id, node: "gamer", started_on: "gamer", place: "stitch", parent: null, parent_title: null,
    detached: true, read_only: false, app: null, state: "idle", outcome: "done", said: "idle", question: null,
    started: ago(3600), last_step: ago(600), model_calls: 1, cost: null, last_words: null, in_flight: null, ...over,
  };
}

test("threads go under the conversation their spawn chain starts from", () => {
  const lanes = lanesOf([
    t("a", { parent: "chat", parent_title: "Orchestrator" }),
    t("b", { parent: "a", state: "working", outcome: null }),
    t("c", { parent: "b" }),
    t("solo", { last_step: ago(60) }),
  ]);
  expect(lanes.map((l) => l.title)).toEqual(["Orchestrator", "solo"]); // working first
  const o = lanes[0]!;
  expect(o.conversation).toBe("chat");
  expect(o.roots.map((s) => s.t.id)).toEqual(["a"]);
  expect(o.roots[0]!.kids[0]!.t.id).toBe("b");
  expect(o.roots[0]!.kids[0]!.kids[0]!.t.id).toBe("c");
  expect(o.counts.working).toBe(1);
  expect(o.live).toBe(true);
});

test("a cycle of parents still shows every thread once", () => {
  const lanes = lanesOf([t("x", { parent: "y" }), t("y", { parent: "x" })]);
  const ids: string[] = [];
  const walk = (s: { t: ThreadItem; kids: any[] }) => { ids.push(s.t.id); s.kids.forEach(walk); };
  lanes.forEach((l) => l.roots.forEach(walk));
  expect(ids.sort()).toEqual(["x", "y"]);
});
