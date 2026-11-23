// `bun test`: thread and effort scopes in the client (design §2.27): staging and
// deduping (src/scopes.ts), the map's multi-pick (map/state.ts `togglePick`),
// a pending message's chips through the outbox and the store's requests, and
// Space's side of it (the chunk's interface; what the rings are drawn on).
import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import type { Api } from "../src/conn";
import type { Request } from "../src/api/Request";
import type { Entry, ThreadItem } from "../src/model";
import { buildModel, DAY } from "../src/map/model";
import * as Sc from "../src/scopes";
import * as O from "../src/transcript/outbox";
import { layoutSpace } from "../src/map/space/layout";
import type { SpaceIO } from "../src/map/space/types";

// What the store reads at import, outside a browser.
const kept = new Map<string, string>();
(globalThis as any).localStorage ??= {
  getItem: (k: string) => kept.get(k) ?? null,
  setItem: (k: string, v: string) => void kept.set(k, v),
  removeItem: (k: string) => void kept.delete(k),
};
const S = await import("../src/store");
const M = await import("../src/map/state");

const NOW = Date.parse("2026-11-13T12:00:00Z");
const iso = (daysAgo: number) => new Date(NOW - daysAgo * DAY).toISOString();
const row = (id: string, o: Partial<ThreadItem> = {}): ThreadItem => ({
  id, title: "T " + id, goal: "", node: "n", started_on: "n", place: "p1", parent: null, parent_title: null, detached: false,
  read_only: false, app: null, state: "idle", outcome: null, said: "idle", question: null, started: iso(2), last_step: iso(1),
  model_calls: 3, cost: 1, last_words: null, in_flight: null, unread: false, ...o,
} as unknown as ThreadItem);
// An effort r → a → b, a lone thread, and a thread whose parent is not loaded.
const rows = [row("r"), row("a", { parent: "r" }), row("b", { parent: "a" }), row("solo", { place: "p2", last_step: iso(0.1) }), row("o", { parent: "gone" })];
const model = () => buildModel(rows, NOW);

describe("staging scopes", () => {
  const eff: Sc.StagedScope = { kind: "effort", id: "r", title: "Root", threads: 3 };
  const thr: Sc.StagedScope = { kind: "thread", id: "x", title: "X" };

  test("each target once; one ID is one target (the latest kind wins, the title kept)", () => {
    expect(Sc.addScopes([eff], [eff, thr, thr])).toEqual([eff, thr]);
    const asThread = Sc.addScopes([eff], [{ kind: "thread", id: "r" }]);
    expect(asThread).toEqual([{ kind: "thread", id: "r", title: "Root" }]);
    expect(Sc.addScopes(asThread, [{ kind: "effort", id: "r" }])).toEqual([{ kind: "effort", id: "r", title: "Root" }]);
  });

  test("toggling adds, then removes; the wire gets kind and id alone", () => {
    const one = Sc.toggleScope([], thr);
    expect(one).toEqual([thr]);
    expect(Sc.toggleScope(one, thr)).toEqual([]);
    expect(Sc.scopeTargets([eff, thr])).toEqual([{ kind: "effort", id: "r" }, { kind: "thread", id: "x" }]);
  });

  test("labels, counts, the prompt sent with scopes alone, and the chips of a pending message", () => {
    expect(Sc.scopeLabel(eff)).toBe("Effort: Root · 3 threads");
    expect(Sc.scopeLabel({ kind: "effort", id: "r" })).toBe("Effort: r");
    expect(Sc.scopeLabel(thr)).toBe("X");
    expect(Sc.scopeThreads([eff, thr])).toBe(4);
    expect(Sc.scopePrompt([eff])).toContain("this effort");
    expect(Sc.scopePrompt([thr])).toContain("this conversation");
    expect(Sc.scopePrompt([eff, thr])).toContain("these attached threads");
    expect(Sc.chipsOf([eff, thr, { kind: "effort", id: "e" }])).toEqual([
      { kind: "effort", id: "r", title: "Root", threads: 3 }, { kind: "thread", id: "x", title: "X", threads: 1 }, { kind: "effort", id: "e", title: null, threads: 0 },
    ]);
  });

  test("a pasted target: effort:<id>, thread:<id>, a bare ID, an anchor; anything else is none", () => {
    const id = "2026-11-13T161824Z-gamer-6bw6";
    expect(Sc.parseScope(`effort:${id}`)).toEqual({ kind: "effort", id });
    expect(Sc.parseScope(` thread:${id} `)).toEqual({ kind: "thread", id });
    expect(Sc.parseScope(id)).toEqual({ kind: "thread", id });
    expect(Sc.parseScope(`${id}#m42`)).toEqual({ kind: "thread", id });
    expect(Sc.parseScope("web client")).toBeNull();
    expect(Sc.parseScope("effort:")).toBeNull();
  });
});

describe("what a thread or a map selection stands for", () => {
  test("an effort with spawned threads is the effort; a lone root, a child, a place are not", () => {
    const m = model();
    expect(Sc.scopeOfSel(m, { kind: "effort", id: "r" })).toEqual({ kind: "effort", id: "r", title: "T r", threads: 3 });
    expect(Sc.scopeOfSel(m, { kind: "effort", id: "solo" })).toEqual({ kind: "thread", id: "solo", title: "T solo" });
    expect(Sc.scopeOfSel(m, { kind: "thread", id: "b" })).toEqual({ kind: "thread", id: "b", title: "T b" });
    expect(Sc.scopeOfSel(m, { kind: "place", id: "p1" })).toBeNull();
    // The synthetic "(parent not loaded)" effort is no thread of the node's.
    const synth = m.threads.find((t) => t.synthetic)!;
    expect(Sc.scopeOfSel(m, { kind: "effort", id: synth.id })).toBeNull();
  });

  test("Ask about this: a thread with spawned threads asks about its whole effort, a leaf about itself", () => {
    const m = model();
    expect(Sc.askScope("a", "T a", m, [])).toEqual({ kind: "effort", id: "r", title: "T r", threads: 3 });
    expect(Sc.askScope("b", "T b", m, [])).toEqual({ kind: "thread", id: "b", title: "T b" });
    // Without the map's forest: the list's rows say whether it has children; the size is the node's to count.
    expect(Sc.askScope("r", "T r", null, rows)).toEqual({ kind: "effort", id: "r", title: "T r" });
    expect(Sc.askScope("b", "T b", null, rows)).toEqual({ kind: "thread", id: "b", title: "T b" });
  });

  test("the picker: words in the title, place or ID, the latest first, an effort offered where there is one", () => {
    const m = model();
    const all = Sc.candidates("", [], m);
    expect(all[0]!.id).toBe("solo");
    expect(all.map((c) => c.id)).not.toContain(m.threads.find((t) => t.synthetic)!.id);
    const b = Sc.candidates("t b", [], m);
    expect(b.map((c) => c.id)).toEqual(["b"]);
    expect(b[0]!.effort).toEqual({ kind: "effort", id: "r", title: "T r", threads: 3 });
    expect(Sc.candidates("p2", [], m).map((c) => [c.id, c.effort])).toEqual([["solo", null]]);
    // From the list alone.
    expect(Sc.candidates("T r", rows, null).map((c) => [c.id, c.effort?.kind ?? null])).toEqual([["r", "effort"]]);
  });
});

describe("the map's multi-pick", () => {
  test("Shift+click toggles an effort or a thread in the tray; the rings' IDs follow; places are not picked", () => {
    M.setRows(rows);
    M.picked.value = [];
    M.togglePick({ kind: "effort", id: "r" });
    M.togglePick({ kind: "thread", id: "b" });
    M.togglePick({ kind: "place", id: "p1" });
    expect(M.picked.value.map((s) => [s.kind, s.id])).toEqual([["effort", "r"], ["thread", "b"]]);
    expect([...M.pickedIds.value]).toEqual(["r", "b"]);
    expect(M.isPicked({ kind: "effort", id: "r" })).toBe(true);
    M.togglePick({ kind: "effort", id: "r" });
    expect(M.picked.value.map((s) => s.id)).toEqual(["b"]);
    expect(M.pickedIds.value.has("r")).toBe(false);
    M.picked.value = [];
  });
});

describe("a message sent with scopes", () => {
  const at = (s: number) => new Date(Date.UTC(2026, 10, 13, 7, 0, s)).toISOString();
  const chips: Sc.StagedScope[] = [{ kind: "effort", id: "r", title: "T r", threads: 3 }];
  const local = (text: string, s: number, outgoing: O.Outgoing): O.UserEntry =>
    ({ n: -1, kind: "user", from: null, at: at(s), text, images: [], scopes: Sc.chipsOf(chips), pending: true, outgoing });

  test("keeps its chips while it waits, is woven, runs and ends; the node's entry replaces it", () => {
    const mine = local("what did it decide?", 5, "sending");
    let list: Entry[] = [mine];
    list = O.mark(list, mine.at, "weaving");
    [list] = O.wove(list, "what did it decide?");
    list = O.ended(O.started(list));
    const still = O.locals(list)[0]!;
    expect(still.scopes).toEqual([{ kind: "effort", id: "r", title: "T r", threads: 3 }]);
    expect(still.outgoing).toBe("running");
    // Read again before the node wrote it: still ours, with its chips.
    expect((O.merge([], O.locals(list))[0] as O.UserEntry).scopes).toEqual(still.scopes);
    // Written: the node's entry (its own chips) stands for it.
    const node: Entry = { n: 3, kind: "user", from: null, at: at(6), text: "what did it decide?", images: [], scopes: [{ kind: "effort", id: "r", title: "T r", threads: 4 }] };
    expect(O.merge([node], O.locals(list))).toEqual([node]);
  });

  test("Send and Chat carry the targets; the bubble shows the chips at once", async () => {
    const sent: Request[] = [];
    const api = {
      call: async (r: Request) => { sent.push(r); return { Lines: ["sent to thread r, on n: no turn runs on it, so it starts a turn"] }; },
      stream: async (r: Request) => { sent.push(r); },
      follow: () => () => {},
      listThreads: async () => ({ threads: rows.map((t) => ({ ...t })), places: [] }),
      thread: async () => null,
      transcript: async () => ({ entries: [], forked_from: null }),
      live: async () => ({ node: "n" }),
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
    await S.select("solo");
    await S.send("what did it decide?", [], true, chips);
    const send = sent.find((r) => typeof r === "object" && "Send" in r) as Extract<Request, { Send: unknown }>;
    expect(send.Send.scopes).toEqual([{ kind: "effort", id: "r" }]);
    const bubble = O.locals(S.entries.value).find((e) => e.text === "what did it decide?")!;
    expect(bubble.scopes).toEqual(Sc.chipsOf(chips));
    expect(S.lastOpen.value).toBe("solo");

    await S.newThread(Sc.scopePrompt(chips), [], chips);
    const chat = sent.find((r) => typeof r === "object" && "Chat" in r && r.Chat.thread === null) as Extract<Request, { Chat: unknown }>;
    expect(chat.Chat.scopes).toEqual([{ kind: "effort", id: "r" }]);
    expect(chat.Chat.text).toContain("Brief me on this effort");
  });
});

describe("Space's side", () => {
  test("the chunk reaches the tray only through its interface: no store, no signals, no scopes module", () => {
    const dir = new URL("../src/map/space/", import.meta.url);
    for (const f of readdirSync(dir).filter((x) => x.endsWith(".ts"))) {
      const src = readFileSync(new URL(f, dir), "utf8");
      const imports = [...src.matchAll(/from\s+"([^"]+)"/g)].map((x) => x[1]!);
      expect([f, imports.filter((i) => /@preact\/signals|store|^\.\.\/state|scopes|actions/.test(i))]).toEqual([f, []]);
    }
    // The hand-over: a plain set and a callback, optional (an older host gives neither).
    const io: Pick<SpaceIO, "picked" | "togglePick"> = { picked: () => new Set(["r"]), togglePick: () => {} };
    expect(io.picked!().has("r")).toBe(true);
  });

  test("every pickable effort and thread has an item to ring", () => {
    const m = model();
    // Islands' packing, roughly: every place and effort a circle.
    const pack = { size: 1000, places: new Map(m.places.map((p) => [p.key, { x: 500, y: 500, r: 450 }])), efforts: new Map(m.efforts.map((e, i) => [e.id, { x: 150 + i * 220, y: 500, r: 90 }])) };
    const L = layoutSpace(m, pack, NOW);
    for (const s of [Sc.scopeOfSel(m, { kind: "effort", id: "r" })!, Sc.scopeOfSel(m, { kind: "thread", id: "b" })!, Sc.scopeOfSel(m, { kind: "effort", id: "solo" })!])
      expect(L.byId.get(s.id)).toBeNumber();
  });
});
