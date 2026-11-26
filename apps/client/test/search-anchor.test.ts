import { expect, test } from "bun:test";
import type { Api } from "../src/conn";
import type { Entry, ThreadItem } from "../src/model";

(globalThis as any).localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };
const S = await import("../src/store");
const A = await import("../src/actions/state");
const at = "2026-11-26T09:00:00Z";
const row = { id: "newest", earlier: ["older"], title: "Conversation", state: "idle" } as ThreadItem;
const message = (n: number, mid: string, id: string | null): Entry =>
  ({ kind: "user", n, id, mid, at, from: null, text: mid, images: [] });
const hit = { thread: "older", msg: "m12", at, title: row.title, sender: "user", snippet: "match", found_by: "words" };

function setup(transcript: Api["transcript"]) {
  S.init({ transcript, onState: () => {}, onThreadsChanged: () => {}, follow: () => () => {}, thread: async () => row } as unknown as Api);
  S.threads.value = [row];
  S.error.value = null;
}

test("search reads back to a compacted message, not a newer one with the same short ID", async () => {
  const pages: Array<number | undefined> = [];
  setup(async (thread, window) => {
    expect(thread).toBe("newest");
    pages.push(window?.before);
    const start = window?.before === undefined ? 600 : window.before - 300;
    const entries = start === 600 ? [message(601, "newest#m12", "m12")]
      : start === 300 ? [message(301, "newest#m1", "m1")]
        : [message(12, "older#m12", null)];
    return { entries, start, earlier: start > 0, forked_from: null, current: "newest" };
  });
  await A.openHit(hit);
  expect(pages).toEqual([undefined, 600, 300]);
  expect(S.selected.value).toBe("newest");
  expect(A.focusedMid()).toBe("older#m12");
  expect(S.error.value).toBeNull();

  await A.openHit({ ...hit, msg: "m999" });
  expect(A.focusedMid()).toBeNull();
  expect(S.error.value).toContain("could not be found");
});

test("a slow hit cannot steal navigation or focus from the next one", async () => {
  let release = () => {};
  const gate = new Promise<void>((r) => { release = r; });
  let reads = 0;
  setup(async () => {
    if (++reads === 1) await gate;
    return { entries: [message(12, "older#m12", null), message(601, "newest#m12", "m12")], start: 0, earlier: false, forked_from: null, current: "newest" };
  });
  const stale = A.openHit(hit);
  await A.openHit({ ...hit, thread: "newest" });
  release();
  await stale;
  expect(A.focusedMid()).toBe("newest#m12");
  expect(S.error.value).toBeNull();
});
