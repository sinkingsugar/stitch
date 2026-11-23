// `bun test` (in apps/client): a compacted conversation reads as one (design
// §2.15, "One conversation"). A thread opened on an old ID opens as the newest;
// a compaction while it is open switches to the new thread; the divider's link
// opens the earlier thread as it was; a message typed there goes to the newest.
import { expect, test } from "bun:test";
import type { Api } from "../src/conn";
import type { Entry, Followed, ThreadItem } from "../src/model";
import type { Request } from "../src/api/Request";
import type { Compaction } from "../src/api/Compaction";

const kept = new Map<string, string>();
(globalThis as any).localStorage ??= {
  getItem: (k: string) => kept.get(k) ?? null,
  setItem: (k: string, v: string) => void kept.set(k, v),
  removeItem: (k: string) => void kept.delete(k),
};
const S = await import("../src/store");

const row = (id: string, title: string, earlier: string[] = []): ThreadItem => ({
  id, title, goal: title, node: "n", started_on: "n", place: "global", parent: null, parent_title: null,
  detached: false, read_only: false, pinned: false, app: null, state: "idle", outcome: null, said: null,
  question: null, started: null, last_step: null, model_calls: 0, cost: null, last_words: null, in_flight: null,
  earlier,
} as unknown as ThreadItem);

const said = (n: number, text: string): Entry =>
  ({ n, kind: "user", from: null, at: "2026-11-09T10:00:00Z", text, images: [] }) as Entry;

const compaction = (from: string, to: string): Compaction => ({
  from, to, model: "anthropic/claude", window: 200_000, tokens: 162_000, kept: 10, summary: "What came before.",
});

test("a compacted conversation is one: old IDs open the newest, and a compaction switches", async () => {
  // The node: "a" was compacted into "b"; "c" is compacted into "d" later.
  let list = [row("b", "Talk", ["a"]), row("c", "Other")];
  const current: Record<string, string> = { a: "b", b: "b", c: "c" };
  const followers = new Map<string, (f: Followed) => void>();
  const sent: Request[] = [];
  const api = {
    call: async (r: Request) => { sent.push(r); return { Lines: ["started"] }; },
    stream: async () => {},
    follow: (thread: string, f: (e: Followed) => void) => { followers.set(thread, f); return () => followers.delete(thread); },
    listThreads: async () => ({ threads: list.map((t) => ({ ...t })), places: [] }),
    thread: async (id: string) => list.find((t) => t.id === current[id]) ?? null,
    transcript: async (id: string) => ({ entries: [said(0, `in ${id}`)], forked_from: null, current: current[id] }),
    live: async () => ({}),
    looseEnds: async () => {},
    schedule: async () => [],
    runsOn: async () => null,
    runners: async () => [],
    runOn: async () => "",
    onState: () => {},
    onThreadsChanged: () => {},
  } as unknown as Api;
  S.init(api);
  await S.refresh();

  // An old ID (a link, a notification) opens the newest, followed as it.
  await S.select("a");
  expect(S.selected.value).toBe("b");
  expect(S.current.value?.id).toBe("b");
  expect(followers.has("b")).toBe(true);
  expect(S.latest.value).toBeNull();

  // One the list does not know as compacted yet: its transcript says where it goes on.
  await S.select("c");
  expect(S.selected.value).toBe("c");
  current.c = "d";
  current.d = "d";
  list = [row("b", "Talk", ["a"]), row("d", "Other", ["c"])];
  // A compaction while it is open: the divider, then the new thread opens.
  followers.get("c")!({ event: { Compacted: compaction("c", "d") } });
  await Bun.sleep(5);
  expect(S.selected.value).toBe("d");
  expect(followers.has("d")).toBe(true);
  await S.refresh();
  expect(S.current.value?.id).toBe("d");

  // The divider's link: the earlier thread as it was, saying where it goes on.
  await S.select("a", true);
  expect(S.selected.value).toBe("a");
  expect(S.latest.value).toBe("b");
  expect(S.current.value?.id).toBe("b");
  expect(S.entries.value.map((e) => (e as { text?: string }).text)).toEqual(["in a"]);

  // Typed there: the newest opens, and the message goes to it.
  await S.send("go on");
  expect(S.selected.value).toBe("b");
  expect(S.latest.value).toBeNull();
  const send = sent.find((r) => typeof r === "object" && "Send" in r) as Extract<Request, { Send: unknown }> | undefined;
  expect(send?.Send.thread).toBe("b");
});

test("the divider says the figures as the node does", () => {
  expect(S.compactedSaid(compaction("a", "b"))).toBe("compacted here (162k of 200k window): summary + last 10 messages");
  expect(S.compactedSaid({ ...compaction("a", "b"), tokens: null, kept: 0 })).toBe("compacted here (past the 200k window): a summary");
});
