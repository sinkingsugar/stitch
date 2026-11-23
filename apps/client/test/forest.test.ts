// The sidebar's spawn trees (src/home/state.ts `forest`): a pinned spawned
// thread is a root of its own, at the top, saying what it hangs under; trees
// go by the latest activity in them.
import { expect, test } from "bun:test";
import { forest } from "../src/home/state";
import { threads } from "../src/home/source";
import type { ThreadItem } from "../src/model";

const ago = (s: number) => new Date(Date.now() - s * 1000).toISOString();
function t(id: string, over: Partial<ThreadItem> = {}): ThreadItem {
  return {
    id, title: id, goal: "Work on " + id, node: "gamer", started_on: "gamer", place: "stitch", parent: null, parent_title: null,
    detached: true, read_only: false, pinned: false, app: null, state: "idle", outcome: "done", said: "idle", question: null,
    started: ago(3600), last_step: ago(600), model_calls: 1, cost: null, last_words: null, in_flight: null, ...over,
  } as ThreadItem;
}

test("a pinned spawned thread is lifted out of its parent's tree, to the top", () => {
  threads.value = [
    t("recent", { last_step: ago(10) }),
    t("chat", { title: "Orchestrator", last_step: ago(5000) }),
    t("kid", { parent: "chat", pinned: true, last_step: ago(4000) }),
    t("other", { parent: "chat", last_step: ago(4500) }),
  ];
  const roots = forest.value;
  expect(roots.map((n) => n.t.id)).toEqual(["kid", "recent", "chat"]);
  expect(roots[0]!.under).toBe("Orchestrator");
  expect(roots[2]!.kids.map((n) => n.t.id)).toEqual(["other"]);
  expect(roots[2]!.under).toBeUndefined();
});

test("a tree moves up with the activity of the threads under it", () => {
  threads.value = [
    t("solo", { last_step: ago(100) }),
    t("chat", { last_step: ago(5000) }),
    t("old", { parent: "chat", last_step: ago(3000) }),
    t("busy", { parent: "chat", last_step: ago(5) }),
  ];
  const roots = forest.value;
  expect(roots.map((n) => n.t.id)).toEqual(["chat", "solo"]);
  expect(roots[0]!.kids.map((n) => n.t.id)).toEqual(["busy", "old"]);
});
