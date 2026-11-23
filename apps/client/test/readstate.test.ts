// Read state (src/readstate.ts, store.ts): the cursor moves only when it
// really advances, by the step the user was shown, never by the clock.
import { expect, test } from "bun:test";
import { readTarget, readThrough, unreadOf } from "../src/readstate";
import type { ThreadItem } from "../src/model";

const kept = new Map<string, string>();
(globalThis as any).localStorage ??= {
  getItem: (k: string) => kept.get(k) ?? null,
  setItem: (k: string, v: string) => void kept.set(k, v),
  removeItem: (k: string) => void kept.delete(k),
};
const S = await import("../src/store");

const T1 = "2026-11-11T09:00:00.000Z";
const T2 = "2026-11-11T09:05:00.000Z";

test("a conversation is read only when unread, its end is in view and the window is focused", () => {
  const row = { unread: true, last_step: T1 };
  expect(readTarget(row, true, true, undefined)).toBe(T1);
  expect(readTarget(row, false, true, undefined)).toBeNull();
  expect(readTarget(row, true, false, undefined)).toBeNull();
  expect(readTarget({ unread: false, last_step: T1 }, true, true, undefined)).toBeNull();
  expect(readTarget({ unread: true, last_step: null }, true, true, undefined)).toBeNull();
});

test("nothing is sent twice for the same step; a newer step is sent", () => {
  expect(readTarget({ unread: true, last_step: T1 }, true, true, T1)).toBeNull();
  expect(readTarget({ unread: true, last_step: T1 }, true, true, T2)).toBeNull();
  expect(readTarget({ unread: true, last_step: T2 }, true, true, T1)).toBe(T2);
});

const row = (id: string, over: Partial<ThreadItem> = {}) => ({ id, unread: true, last_step: T1, ...over }) as ThreadItem;

test("reading through a step leaves a newer step unread", () => {
  const out = readThrough([row("a"), row("b", { last_step: T2 }), row("c", { unread: false })], new Map([["a", T1], ["b", T1]]));
  expect(out.map((t) => t.unread)).toEqual([false, true, false]);
  expect(unreadOf(out).map((x) => x.id)).toEqual(["b"]);
});

function fakeApi(sent: unknown[], list: () => ThreadItem[] = () => []) {
  return {
    call: async (r: unknown) => { sent.push(r); return "Ok"; },
    stream: async () => {}, follow: () => () => {},
    listThreads: async () => ({ threads: list(), places: [] }), placeThreads: async (p: string) => S.placeRows.value[p] ?? [],
    thread: async () => null, transcript: async () => ({ entries: [], forked_from: null }),
    live: async () => ({}), looseEnds: async () => {}, schedule: async () => [], runsOn: async () => null,
    runners: async () => [], runOn: async () => "", onState: () => {}, onThreadsChanged: () => {},
  } as any;
}

test("marking read writes once per advance, and the list shows it at once", async () => {
  const sent: any[] = [];
  S.init(fakeApi(sent));
  S.threads.value = [row("x"), row("y", { last_step: T2 })];
  await S.markRead([{ id: "x", upto: T1 }]);
  await S.markRead([{ id: "x", upto: T1 }]); // the same step again: nothing
  await S.markRead([{ id: "x", upto: "2001-01-01T00:00:00Z" }]); // older: nothing
  expect(sent.map((r) => r.Read)).toEqual([{ thread: "x", upto: T1 }]);
  expect(S.threads.value.map((t) => t.unread)).toEqual([false, true]);
  // "Mark all read" of what is unread: only y.
  await S.markUnreadIn(S.threads.value);
  expect(sent.map((r) => r.Read.thread)).toEqual(["x", "y"]);
  expect(S.threads.value.every((t) => !t.unread)).toBe(true);
});

test("mark all read goes to the node (with the place, when given) and shows at once, loaded rows or not", async () => {
  const sent: any[] = [];
  S.init(fakeApi(sent, () => S.threads.value));
  S.threads.value = [row("p", { place: "stitch" }), row("q", { place: "otter" })];
  S.placeRows.value = { stitch: [row("old", { place: "stitch" })] };
  await S.markAllRead("stitch");
  expect(sent.map((r) => r.ReadAll)).toEqual([{ place: "stitch" }]);
  expect(S.threads.value.map((t) => t.unread)).toEqual([false, true]);
  expect(S.placeRows.value.stitch!.map((t) => t.unread)).toEqual([false]);
  await S.markAllRead();
  expect(sent.map((r) => r.ReadAll)).toEqual([{ place: "stitch" }, { place: null }]);
  expect(S.threads.value.every((t) => !t.unread)).toBe(true);
});
