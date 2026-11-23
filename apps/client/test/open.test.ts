// `bun test` (in apps/client): opening a thread older than the list's window
// (a search hit) shows it, and a refresh of the list does not lose it
// (src/store.ts `extra`, `Threads { thread }`).
import { expect, test } from "bun:test";
import type { Api } from "../src/conn";
import type { ThreadItem } from "../src/model";

// What the store reads at import, outside a browser.
const kept = new Map<string, string>();
(globalThis as any).localStorage ??= {
  getItem: (k: string) => kept.get(k) ?? null,
  setItem: (k: string, v: string) => void kept.set(k, v),
  removeItem: (k: string) => void kept.delete(k),
};
const S = await import("../src/store");

const row = (id: string, title: string): ThreadItem => ({
  id, title, goal: title, node: "n", started_on: "n", place: "global", parent: null, parent_title: null,
  detached: true, read_only: false, pinned: false, app: null, state: "idle", outcome: null, said: null,
  question: null, started: null, last_step: null, model_calls: 0, cost: null, last_words: null, in_flight: null,
} as unknown as ThreadItem);

test("a thread outside the list's window is shown, and stays through refreshes", async () => {
  const window = [row("new-1", "New one")];
  const older: Record<string, ThreadItem> = { "old-1": row("old-1", "Beaming WiFi to Tesla") };
  let asked: string[] = [];
  let release: () => void = () => {};
  const gate = new Promise<void>((r) => (release = r));
  const api = {
    call: async () => "Ok",
    stream: async () => {},
    follow: () => () => {},
    listThreads: async () => ({ threads: window.map((t) => ({ ...t })), places: [] }),
    thread: async (id: string) => { asked.push(id); await gate; return older[id] ?? null; },
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
  // Another test file may have left a thread open (the store is a module's signals).
  S.selected.value = null;
  S.init(api);
  await S.refresh();
  expect(S.threads.value.map((t) => t.id)).toEqual(["new-1"]);

  const opening = S.select("old-1");
  // While its row is read: the thread is open (not the home view), no row yet.
  await Bun.sleep(0);
  expect(S.homeView.value).toBe(false);
  expect(S.selected.value).toBe("old-1");
  expect(S.current.value).toBeNull();
  release();
  await opening;
  expect(asked).toContain("old-1");
  expect(S.current.value?.title).toBe("Beaming WiFi to Tesla");

  // The list's refresh (still without it) keeps it open, its row read again.
  asked = [];
  older["old-1"] = { ...older["old-1"]!, state: "working" } as ThreadItem;
  await S.refresh();
  expect(asked).toEqual(["old-1"]);
  expect(S.current.value?.state).toBe("working");

  // A thread in the list is not read on its own.
  asked = [];
  await S.select("new-1");
  expect(asked).toEqual([]);
  expect(S.current.value?.title).toBe("New one");

  // One the node does not know: said so, not the home view.
  await S.select("gone-1");
  expect(S.homeView.value).toBe(false);
  expect(S.current.value).toBeNull();
  expect(S.extra.value["gone-1"]).toBeNull();
});
