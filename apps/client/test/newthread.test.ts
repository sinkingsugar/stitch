// `bun test` (in apps/client): a new thread is shown working from the moment
// the node names it, though the node writes it (and lists it) only with its
// first step, which a long first answer makes a minute away (src/store.ts
// `provisional`). Before, the view said "No such thread" over the message,
// with no sign of the turn, until that step ended.
import { expect, test } from "bun:test";
import type { Api } from "../src/conn";
import type { ThreadItem } from "../src/model";

const kept = new Map<string, string>();
(globalThis as any).localStorage ??= {
  getItem: (k: string) => kept.get(k) ?? null,
  setItem: (k: string, v: string) => void kept.set(k, v),
  removeItem: (k: string) => void kept.delete(k),
};
const S = await import("../src/store");

const row = (id: string, title: string, state = "idle"): ThreadItem => ({
  id, title, goal: title, node: "n", started_on: "n", place: "global", parent: null, parent_title: null,
  detached: false, read_only: false, pinned: false, app: null, state, outcome: null, said: state,
  question: null, started: null, last_step: null, model_calls: 0, cost: null, last_words: null, in_flight: null,
} as unknown as ThreadItem);

test("a new thread is shown working before its first step is written", async () => {
  const listed: ThreadItem[] = [];
  const written: Record<string, ThreadItem> = {};
  let finish: () => void = () => {};
  const stepDone = new Promise<void>((r) => (finish = r));
  const api = {
    call: async () => "Ok",
    stream: async (_req: unknown, onReply: (r: unknown) => void) => {
      onReply({ Thread: "t-new" });
      await stepDone;
      onReply({ ChatDone: { thread: "t-new" } });
    },
    follow: () => () => {},
    listThreads: async () => ({ threads: listed.map((t) => ({ ...t })), places: [] }),
    thread: async (id: string) => written[id] ?? null,
    transcript: async () => ({ entries: [], forked_from: null }),
    live: async () => ({}),
    looseEnds: async () => {},
    schedule: async () => [],
    runsOn: async () => null,
    runners: async () => [],
    runOn: async () => "",
    onState: () => {},
    onThreadsChanged: () => {},
  } as unknown as Api;
  S.selected.value = null;
  S.init(api);
  await S.refresh();

  const sending = S.newThread("Why is it slow?\nmore detail");
  for (let i = 0; i < 20 && S.selected.value !== "t-new"; i++) await Bun.sleep(1);
  await Bun.sleep(5);
  // Named, not written: open, working, titled from the message, its turn shown.
  expect(S.selected.value).toBe("t-new");
  expect(S.current.value?.state).toBe("working");
  expect(S.current.value?.title).toBe("Why is it slow?");
  expect(S.turn.value).not.toBeNull();
  // A refresh of the list (still without it) keeps it so.
  await S.refresh();
  expect(S.current.value?.state).toBe("working");

  // The step is written and the turn ends: the node's row from then on.
  written["t-new"] = row("t-new", "Why is it slow?");
  listed.push(written["t-new"]);
  finish();
  await sending;
  await Bun.sleep(5);
  expect(S.current.value?.state).toBe("idle");
});

test("a new thread that fails before writing anything is not left working", async () => {
  const api = {
    call: async () => "Ok",
    stream: async (_req: unknown, onReply: (r: unknown) => void) => {
      onReply({ Thread: "t-failed" });
      await Bun.sleep(5);
      throw new Error("model refused");
    },
    follow: () => () => {},
    listThreads: async () => ({ threads: [], places: [] }),
    thread: async () => null,
    transcript: async () => ({ entries: [], forked_from: null }),
    live: async () => ({}),
    looseEnds: async () => {},
    schedule: async () => [],
    runsOn: async () => null,
    runners: async () => [],
    runOn: async () => "",
    onState: () => {},
    onThreadsChanged: () => {},
  } as unknown as Api;
  S.selected.value = null;
  S.init(api);
  await S.newThread("hello");
  await Bun.sleep(5);
  expect(S.selected.value).toBe("t-failed");
  expect(S.current.value).toBeNull();
  expect(S.error.value).toContain("model refused");
});
