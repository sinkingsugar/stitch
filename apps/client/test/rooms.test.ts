// `bun test`: rooms in the web client (design §2.27, rooms.md §4, §5): the
// outbox's reading of a room's `Send`, `@slug` mentions, speaker colours and
// notes, the payloads, `RoomCreate` ("Pull in model…", a new room, the map's
// picks), `RoomUpdate`, the store following a round, and the mock's room.
import { describe, expect, test } from "bun:test";
import type { Api } from "../src/conn";
import type { Request } from "../src/api/Request";
import type { Entry, ThreadItem } from "../src/model";
import type { RoomInfo, RoomParticipant, Transcript } from "../src/payload";
import * as L from "../src/room/logic";
import * as O from "../src/transcript/outbox";

// What the store reads at import, outside a browser.
const kept = new Map<string, string>();
(globalThis as any).localStorage ??= {
  getItem: (k: string) => kept.get(k) ?? null,
  setItem: (k: string, v: string) => void kept.set(k, v),
  removeItem: (k: string) => void kept.delete(k),
};
const S = await import("../src/store");
const R = await import("../src/room/state");
const { MockConn, ROOM } = await import("../src/mock");

const roster: RoomParticipant[] = [
  { slug: "opus", name: "Opus", model: "anthropic/claude-opus-5-5", thread: "t-opus", muted: false },
  { slug: "gemini", name: "Gemini", model: "google/gemini-3-pro", thread: "t-gem", muted: false },
  { slug: "gpt", name: "GPT", model: "openai/gpt-6-astra", thread: "t-gpt", muted: true },
  { slug: "gpt-2", name: "Big Model", model: "openai/gpt-6-astra", thread: "t-gpt2", muted: false },
];
const info: RoomInfo = { room: "room-1", title: "R", promoted_from: "t-opus", budget_usd: null, mode: "sequential", hop_cap: 2, participants: roster, running: false };

describe("the outbox in a room", () => {
  test("a round starts on it, or the round running takes it; other replies as before", () => {
    expect(O.sentAs("a round runs on it", false)).toBe("starting");
    expect(O.sentAs("a round runs on it", true)).toBe("starting");
    expect(O.sentAs("taken by the round running in the room", true)).toBe("weaving");
    expect(O.sentAs("taken by the round running in the room", false)).toBe("weaving");
    expect(O.sentAs("sent to thread x: woven into its running turn", false)).toBe("weaving");
    expect(O.sentAs(undefined, true)).toBe("queued");
  });

  test("the speaker streaming goes on above a message not taken yet, and after it once taken", () => {
    const mine = { ...S.optimistic("Actually, second?", []), outgoing: "weaving" as const };
    let list: Entry[] = [{ kind: "live-assistant", text: "Opus wri", speaker: roster[0]! }, mine];
    list = O.stream(list, { kind: "live-assistant", text: "tes" }, "tes");
    expect(list.map((e) => e.kind)).toEqual(["live-assistant", "user"]);
    expect((list[0] as { text: string }).text).toBe("Opus writes");
    // The room took it (its note): the next speaker's bubble goes after it.
    const [taken, ours] = O.wove(list, "Actually, second?");
    expect(ours).toBe(true);
    list = O.stream(taken, { kind: "live-assistant", text: "Gemini", speaker: roster[1]! }, "Gemini");
    expect(list.map((e) => e.kind)).toEqual(["live-assistant", "user", "live-assistant"]);
    expect(O.said(list[1] as O.UserEntry, true)).toBeNull();
  });

  test("its lines say round, not turn; the woven one starts its own round when the round ends", () => {
    const e = S.optimistic("hello room", []);
    expect(O.said({ ...e, outgoing: "starting" }, true)).toContain("round");
    expect(O.said({ ...e, outgoing: "weaving" }, true)).toContain("after the current speaker");
    expect(O.said({ ...e, outgoing: "starting" })).toBe("sent · starting a turn…");
    const after = O.ended([{ ...e, outgoing: "weaving" }]);
    expect((after[0] as O.UserEntry).outgoing).toBe("starting");
  });
});

describe("@slug mentions", () => {
  test("found at a word's start up to the caret, not inside an address", () => {
    expect(L.mentionAt("hi @ge", 6)).toEqual({ start: 3, query: "ge" });
    expect(L.mentionAt("@", 1)).toEqual({ start: 0, query: "" });
    expect(L.mentionAt("(@op", 4)).toEqual({ start: 1, query: "op" });
    expect(L.mentionAt("mail me at gio@example", 22)).toBeNull();
    expect(L.mentionAt("hi @ge there", 12)).toBeNull();
    expect(L.mentionAt("hi @gem there", 7)).toEqual({ start: 3, query: "gem" });
  });

  test("handles first, then names without spaces, case aside; roster order within a rank", () => {
    expect(L.mentionMatches("", roster).map((p) => p.slug)).toEqual(["opus", "gemini", "gpt", "gpt-2"]);
    expect(L.mentionMatches("g", roster).map((p) => p.slug)).toEqual(["gemini", "gpt", "gpt-2"]);
    expect(L.mentionMatches("GP", roster).map((p) => p.slug)).toEqual(["gpt", "gpt-2"]);
    expect(L.mentionMatches("bigm", roster).map((p) => p.slug)).toEqual(["gpt-2"]);
    expect(L.mentionMatches("mini", roster).map((p) => p.slug)).toEqual(["gemini"]);
    expect(L.mentionMatches("zz", roster)).toEqual([]);
  });

  test("inserting replaces what was typed, one space after, the caret past it", () => {
    expect(L.insertMention("hi @ge", 3, 6, "gemini")).toEqual({ text: "hi @gemini ", caret: 11 });
    expect(L.insertMention("hi @ge what?", 3, 6, "gemini")).toEqual({ text: "hi @gemini what?", caret: 11 });
    expect(L.insertMention("@", 0, 1, "opus")).toEqual({ text: "@opus ", caret: 6 });
  });

  test("who a message addresses, as the node schedules it: handles or names, in order, once", () => {
    expect(L.addressed("what do you think?", roster)).toEqual([]);
    expect(L.addressed("@gemini then @opus, and @gemini again", roster).map((p) => p.slug)).toEqual(["gemini", "opus"]);
    expect(L.addressed("@BigModel please.", roster).map((p) => p.slug)).toEqual(["gpt-2"]);
    expect(L.addressed("mail gio@opus.com", roster)).toEqual([]);
    expect(L.addressed("@nobody", roster)).toEqual([]);
  });
});

describe("speakers", () => {
  test("a slug's colour is the same every time, and differs between handles", () => {
    expect(L.speakerColor("opus")).toBe(L.speakerColor("opus"));
    expect(L.speakerColor("Opus")).toBe(L.speakerColor("opus"));
    expect(L.speakerHue("opus")).toBeGreaterThanOrEqual(0);
    expect(L.speakerHue("opus")).toBeLessThan(360);
    const hues = new Set(["opus", "gemini", "gpt", "sonnet", "grok"].map(L.speakerHue));
    expect(hues.size).toBe(5);
    expect(L.speakerColor("gemini")).toMatch(/^hsl\(\d+ 70% 66%\)$/);
  });

  test("models: short, provider; the pill's line with a node elsewhere and the spend", () => {
    expect(L.shortModel("anthropic/claude-opus-5-5")).toBe("claude-opus-5-5");
    expect(L.shortModel("default")).toBe("default");
    expect(L.providerOf("google/gemini-3-pro")).toBe("google");
    const row = { node: "mbp", cost: 0.214 } as ThreadItem;
    expect(L.pillMeta(roster[1]!, row, "gamer")).toBe("gemini-3-pro · @mbp · $0.21");
    expect(L.pillMeta(roster[1]!, { ...row, node: "gamer", cost: null } as ThreadItem, "gamer")).toBe("gemini-3-pro");
  });

  test("the room's notes, as rooms.rs streams them", () => {
    expect(L.roomNote("[Gemini · google/gemini-3-pro]", roster)).toEqual({ kind: "speaker", speaker: { slug: "gemini", name: "Gemini", thread: "t-gem", model: "google/gemini-3-pro" } });
    expect(L.roomNote("[Big Model · openai/gpt-6-astra]", roster)).toMatchObject({ kind: "speaker", speaker: { slug: "gpt-2" } });
    expect(L.roomNote("[GPT passed]", roster)).toMatchObject({ kind: "pass", slug: "gpt", name: "GPT" });
    expect(L.roomNote("[Opus was stopped]", roster)).toMatchObject({ kind: "stopped", slug: "opus" });
    expect(L.roomNote("[Opus said nothing]", roster)).toMatchObject({ kind: "silent" });
    expect(L.roomNote("[Opus did not answer: no key]", roster)).toMatchObject({ kind: "failed", slug: "opus" });
    expect(L.roomNote("[The room stops: the budget is spent.]", roster)).toMatchObject({ kind: "room" });
    expect(L.roomNote("[Everyone in the room is muted: nobody answers. Unmute someone, or @mention them.]", roster)).toMatchObject({ kind: "room" });
    expect(L.roomNote("[The room's round takes your message after the participant speaking now.]", roster)).toMatchObject({ kind: "room" });
    // Not a participant's name, or another note.
    expect(L.roomNote("[Someone · x/y]", roster)).toBeNull();
    expect(L.roomNote("[passed on to mbp]", roster)).toBeNull();
    expect(L.quietNote(L.roomNote("[GPT passed]", roster)!)).toBe("GPT passed");
  });

  test("payloads: a room row, a transcript's room, a reply's speaker and a relayed message parse as typed", () => {
    const t: Transcript = JSON.parse(JSON.stringify({
      thread: "room-1", next: 2, forked_from: null, current: "room-1", room: info,
      entries: [
        { n: 0, kind: "user", from: "room:seed", at: "x", text: "seed", images: [], scopes: [{ kind: "thread", id: "t-opus", title: "T", threads: 1 }] },
        { n: 1, kind: "assistant", from: "participant:gemini:t-gem", at: "x", text: "hi", images: [], speaker: { slug: "gemini", name: "Gemini", thread: "t-gem", model: "google/gemini-3-pro" } },
      ],
    }));
    expect(t.room?.participants.map((p) => p.slug)).toEqual(["opus", "gemini", "gpt", "gpt-2"]);
    const e = t.entries[1]!;
    expect(e.kind === "assistant" && e.speaker?.thread).toBe("t-gem");
    const relayed: Entry = { n: 0, kind: "user", from: "room room-1", at: "x", text: "[User]: hi", images: [], room_turn: { room: "room-1", upto: "m3", slug: "gemini" } };
    expect(relayed.kind === "user" && relayed.room_turn?.room).toBe("room-1");
    const rows = [{ id: "room-1", room: true } as ThreadItem, { id: "t-gem", parent: "room-1" } as ThreadItem];
    expect(L.roomOfThread(rows[1]!, rows)).toBe("room-1");
    expect(L.roomOfThread({ id: "t-opus", parent: null } as ThreadItem, rows, "room-1")).toBe("room-1");
    expect(L.roomOfThread({ id: "x", parent: null } as ThreadItem, rows)).toBeNull();
    expect(L.rowOf({ thread: "old" }, [{ id: "new", earlier: ["old"] } as ThreadItem])?.id).toBe("new");
  });
});

describe("participants' questions in the room", () => {
  const row = (id: string, o: Partial<ThreadItem> = {}) => ({ id, earlier: [], question: null, ...o }) as unknown as ThreadItem;
  const ask = "Opus (read-only, in room “R”) wants to write apps/pot-feet/index.html";

  test("a settled question's note is a quiet note of its participant", () => {
    expect(L.roomNote("[Opus was allowed to write a.txt]", roster)).toMatchObject({ kind: "asked", outcome: "allowed", slug: "opus", name: "Opus" });
    expect(L.roomNote("[Opus was allowed to write a.txt, and made writable]", roster)).toMatchObject({ kind: "asked", outcome: "always" });
    expect(L.roomNote("[Big Model was allowed to run `git push`, and from now on: git push]", roster)).toMatchObject({ kind: "asked", outcome: "always", slug: "gpt-2" });
    expect(L.roomNote("[Gemini was not allowed to write b.txt: not that one]", roster)).toMatchObject({ kind: "asked", outcome: "declined", slug: "gemini" });
    expect(L.roomNote("[Gemini no longer asks to write c.txt]", roster)).toMatchObject({ kind: "asked", outcome: "withdrawn" });
    expect(L.roomNote("[Nobody was allowed to write x]", roster)).toBeNull();
    expect(L.quietNote(L.roomNote("[Gemini was not allowed to write b.txt: not that one]", roster)!)).toBe("Gemini was not allowed to write b.txt: not that one");
    // Kept in the room's transcript as a note entry.
    const t: Transcript = JSON.parse(JSON.stringify({ thread: "room-1", next: 1, forked_from: null, current: "room-1", room: info, entries: [{ n: 0, kind: "note", at: "x", text: "[Opus was allowed to write a.txt]" }] }));
    const e = t.entries[0]!;
    expect(e.kind === "note" && e.text).toBe("[Opus was allowed to write a.txt]");
  });

  test("a participant's question: from its row, its words without the room's aside", () => {
    const rows = [row("room-1", { room: true } as Partial<ThreadItem>), row("t-opus", { question: { id: "q1", command: ask } }), row("t-gem")];
    const asks = L.roomAsks(roster, rows);
    expect(asks.map((a) => [a.participant.slug, a.question.id, a.text, a.what])).toEqual([["opus", "q1", "Opus wants to write apps/pot-feet/index.html", null]]);
    expect(L.waitingLine(asks)).toBe("Opus is waiting for your approval");
    // A command: its words apart.
    expect(L.askText(roster[1]!, "git push origin main")).toEqual({ text: "Gemini wants to run", what: "git push origin main" });
    // Another room's participant's aside is not taken for this one's name.
    expect(L.askText(roster[1]!, ask).what).toBe(ask);
  });

  test("from the attention list too: any thread of the participant's chain, what always allows", () => {
    const rows = [row("t-gem-2", { earlier: ["t-gem"] })];
    const qs = [
      { id: "q2", command: "rm -rf build", thread: "t-gem-2", grant: ["rm -rf build"] },
      { id: "q3", command: "ls", thread: "someone-else", grant: [] },
      { id: "q4", command: "Big Model (read-only, in room “R”) wants to write x", thread: "t-gpt2", grant: ["make Big Model writable"] },
    ];
    const asks = L.roomAsks(roster, rows, qs);
    expect(asks.map((a) => [a.participant.slug, a.question.id, a.text])).toEqual([["gemini", "q2", "Gemini wants to run"], ["gpt-2", "q4", "Big Model wants to write x"]]);
    expect(asks[1]!.question.grant).toEqual(["make Big Model writable"]);
    expect(L.waitingLine(asks)).toBe("Gemini and Big Model are waiting for your approval");
    // The row's question wins, the list's grant with it.
    const both = L.roomAsks(roster, [row("t-opus", { question: { id: "q5", command: ask } })], [{ id: "q5", command: ask, thread: "t-opus", grant: ["make Opus writable"] }]);
    expect(both.map((a) => [a.question.id, a.question.grant])).toEqual([["q5", ["make Opus writable"]]]);
    expect(L.roomAsks(roster, [], [])).toEqual([]);
    expect(L.waitingLine([])).toBeNull();
  });
});

describe("the requests", () => {
  test("Pull in model…: the thread first (from_thread, its place), the new models read-only by default", () => {
    const r = L.roomCreate({ from: "t1", place: "project:x", models: [{ model: "google/gemini-3-pro" }, { model: "openai/gpt-6-astra", access: "no-tools", node: "mbp" }], threads: ["t1"] });
    expect(r).toEqual({
      RoomCreate: {
        title: null, place: null, from_thread: "t1",
        participants: [
          { model: "google/gemini-3-pro", thread: null, name: null, slug: null, node: null, tools: true, read_only: true },
          { model: "openai/gpt-6-astra", thread: null, name: null, slug: null, node: "mbp", tools: false, read_only: true },
        ],
        scopes: [], mode: null, hop_cap: null, budget_usd: null,
      },
    });
    expect(L.participantCount({ from: "t1", models: [{ model: "m" }], threads: ["t1"] })).toBe(2);
  });

  test("a new room: models writable when asked, invited threads as they are, scopes, policy, budget", () => {
    const r = L.roomCreate({ title: "  Council ", place: "project:stitch", models: [{ model: "a/b", access: "writable" }], threads: ["t9"], scopes: [{ kind: "effort", id: "e1", title: "E", threads: 4 }], mode: "parallel", hopCap: 0, budgetUsd: 2.5 });
    expect(r).toEqual({
      RoomCreate: {
        title: "Council", place: "project:stitch", from_thread: null,
        participants: [
          { model: null, thread: "t9", name: null, slug: null, node: null, tools: true, read_only: false },
          { model: "a/b", thread: null, name: null, slug: null, node: null, tools: true, read_only: false },
        ],
        scopes: [{ kind: "effort", id: "e1" }], mode: "parallel", hop_cap: 0, budget_usd: 2.5,
      },
    });
  });

  test("RoomUpdate: every list present, only what changes set; shown at once", () => {
    expect(L.roomUpdate("room-1", { mute: ["gpt"] })).toEqual({ RoomUpdate: { thread: "room-1", add: [], remove: [], mute: ["gpt"], unmute: [], mode: null, hop_cap: null } });
    expect(L.roomUpdate("room-1", { mode: "parallel", hopCap: 3, add: [L.threadSpec("t7")], remove: ["gpt-2"] })).toEqual({
      RoomUpdate: { thread: "room-1", add: [{ model: null, thread: "t7", name: null, slug: null, node: null, tools: true, read_only: false }], remove: ["gpt-2"], mute: [], unmute: [], mode: "parallel", hop_cap: 3 },
    });
    const next = L.applyChange(info, { mute: ["opus"], unmute: ["gpt"], remove: ["gpt-2"], mode: "parallel", hopCap: 0 });
    expect(next.participants.map((p) => [p.slug, p.muted])).toEqual([["opus", true], ["gemini", false], ["gpt", false]]);
    expect([next.mode, next.hop_cap]).toEqual(["parallel", 0]);
    expect(info.participants).toHaveLength(4);
  });

  test("second opinions: another provider than the thread's, the first of each, then the rest", () => {
    const models = ["anthropic/claude-opus-5-5", "anthropic/claude-sonnet-5-5", "openai/gpt-6-astra", "google/gemini-3-pro", "mock/mock"];
    expect(L.secondOpinions("anthropic/claude-opus-5-5", models)).toEqual(["openai/gpt-6-astra"]);
    expect(L.secondOpinions("anthropic/claude-opus-5-5", models, 3)).toEqual(["openai/gpt-6-astra", "google/gemini-3-pro", "anthropic/claude-sonnet-5-5"]);
    expect(L.secondOpinions("openai/gpt-6-astra", models, 2)).toEqual(["anthropic/claude-opus-5-5", "google/gemini-3-pro"]);
    expect(L.secondOpinions(null, ["anthropic/a"], 2)).toEqual(["anthropic/a"]);
  });

  test("a reply that is no room's is refused", () => {
    expect(() => L.infoOf("Ok")).toThrow();
    expect(L.infoOf({ Lines: [JSON.stringify(info)] }).room).toBe("room-1");
  });
});

describe("the store with a room", () => {
  const row = (id: string, o: Partial<ThreadItem> = {}): ThreadItem => ({
    id, title: "T " + id, goal: "", node: "n", started_on: "n", place: "p1", parent: null, parent_title: null, detached: false,
    read_only: false, app: null, state: "idle", outcome: null, said: "idle", question: null, started: "2026-11-13T10:00:00Z", last_step: "2026-11-13T11:00:00Z",
    model_calls: 3, cost: 1, last_words: null, in_flight: null, unread: false, ...o,
  } as ThreadItem);
  const rows = [row("t-opus"), row("room-1", { room: true, participants: roster, room_mode: "sequential" }), row("t-gem", { parent: "room-1" })];
  const sent: Request[] = [];
  let room: RoomInfo = structuredClone(info);
  const api = {
    call: async (r: Request) => {
      sent.push(r);
      if (typeof r === "object" && "RoomCreate" in r) return { Lines: [JSON.stringify(room)] };
      if (typeof r === "object" && "RoomUpdate" in r) { room = L.applyChange(room, { mute: r.RoomUpdate.mute, mode: (r.RoomUpdate.mode ?? undefined) as RoomInfo["mode"] | undefined }); return { Lines: [JSON.stringify(room)] }; }
      if (typeof r === "object" && "Send" in r) return { Lines: ["a round runs on it"] };
      return "Ok";
    },
    stream: async (r: Request) => { sent.push(r); },
    follow: () => () => {},
    listThreads: async () => ({ threads: rows.map((t) => ({ ...t })), places: [] }),
    thread: async () => null,
    transcript: async (id: string) => ({ entries: [], forked_from: null, room: id === "room-1" ? room : null }),
    live: async () => ({ node: "n", places: [] }),
    looseEnds: async () => {},
    schedule: async () => [],
    runsOn: async () => null,
    runners: async () => [],
    runOn: async () => "",
    onState: () => {},
    onThreadsChanged: () => {},
  } as unknown as Api;

  test("Pull in model… makes the room and opens it, its roster known", async () => {
    S.selected.value = null;
    S.init(api);
    await S.refresh();
    const made = await R.create({ from: "t-opus", models: [{ model: "google/gemini-3-pro" }] });
    expect(made?.room).toBe("room-1");
    expect(S.selected.value).toBe("room-1");
    expect(S.room.value?.participants.map((p) => p.slug)).toEqual(["opus", "gemini", "gpt", "gpt-2"]);
    const create = sent.find((r) => typeof r === "object" && "RoomCreate" in r) as Extract<Request, { RoomCreate: unknown }>;
    expect(create.RoomCreate.from_thread).toBe("t-opus");
    expect(create.RoomCreate.participants[0]!.read_only).toBe(true);
  });

  test("the map's picks open a room: scopes attached, picked threads invited", () => {
    const r = L.roomCreate({ place: "p1", scopes: [{ kind: "effort", id: "e1" }, { kind: "thread", id: "t5" }], threads: ["t5"], models: [{ model: "a/b" }] });
    const c = (r as Extract<Request, { RoomCreate: unknown }>).RoomCreate;
    expect(c.scopes).toEqual([{ kind: "effort", id: "e1" }, { kind: "thread", id: "t5" }]);
    expect(c.participants.map((p) => p.thread ?? p.model)).toEqual(["t5", "a/b"]);
  });

  test("muting and the policy switch: shown at once, then as the node says", async () => {
    const p = R.update("room-1", { mute: ["gemini"] });
    expect(S.room.value?.participants.find((x) => x.slug === "gemini")?.muted).toBe(true);
    await p;
    expect(S.room.value?.participants.find((x) => x.slug === "gemini")?.muted).toBe(true);
    await R.update("room-1", { mode: "parallel" });
    expect(S.room.value?.mode).toBe("parallel");
    const u = sent.filter((r) => typeof r === "object" && "RoomUpdate" in r);
    expect(u).toHaveLength(2);
  });

  test("a message to the room: Send, then starting; a round's notes open each speaker's bubble, a pass is a quiet note", async () => {
    room = { ...room, mode: "sequential" };
    S.room.value = room;
    await S.send("@gemini and @opus: thoughts?", [], true);
    const bubble = O.locals(S.entries.value).find((e) => e.text === "@gemini and @opus: thoughts?")!;
    expect(bubble.outgoing).toBe("starting");
    S.apply("room-1", "Started");
    expect((O.locals(S.entries.value)[0] as O.UserEntry).outgoing).toBe("running");
    S.apply("room-1", { Note: "[Gemini · google/gemini-3-pro]" });
    expect(S.speaking.value?.slug).toBe("gemini");
    S.apply("room-1", { Text: "Mine." });
    S.apply("room-1", { Text: "\n" });
    S.apply("room-1", { Note: "[Opus · anthropic/claude-opus-5-5]" });
    S.apply("room-1", { Text: "Agreed." });
    // A question of Opus's settled mid-turn: a quiet note, Opus still speaks.
    S.apply("room-1", { Note: "[Opus was allowed to write a.txt]" });
    expect(S.speaking.value?.slug).toBe("opus");
    S.entries.value = S.entries.value.filter((e) => !(e.kind === "note" && e.text.startsWith("[Opus was")));
    S.apply("room-1", { Note: "[GPT passed]" });
    const live = S.entries.value.filter((e): e is Extract<Entry, { kind: "live-assistant" }> => e.kind === "live-assistant");
    expect(live.map((e) => [e.speaker?.slug, e.text.trim()])).toEqual([["gemini", "Mine."], ["opus", "Agreed."]]);
    expect(S.entries.value.at(-1)).toEqual({ kind: "note", text: "[GPT passed]" });
    S.apply("room-1", "Ended");
    expect(S.speaking.value).toBeNull();
    // Cancel stops the room's round: the room's own Cancel.
    await S.cancel();
    expect(sent.at(-1)).toEqual({ Cancel: { thread: "room-1" } });
    await R.stopOne("t-gem");
    expect(sent.at(-1)).toEqual({ Cancel: { thread: "t-gem" } });
  });
});

describe("the mock's room (?mock)", () => {
  test("RoomInfo, RoomUpdate, Send and RoomCreate answer as the node does", async () => {
    const m = new MockConn();
    const read = async (r: Request) => L.infoOf(await m.call(r));
    const i = await read({ RoomInfo: { thread: ROOM } });
    expect(i.participants.map((p) => p.slug)).toEqual(["opus", "gemini", "gpt"]);
    expect(i.promoted_from).toBeTruthy();
    const t = await m.transcript(ROOM);
    expect(t.room?.room).toBe(ROOM);
    expect(t.entries.filter((e) => e.kind === "assistant" && e.speaker).length).toBeGreaterThanOrEqual(4);
    expect(t.entries.some((e) => e.kind === "note" && /passed\]$/.test(e.text))).toBe(true);
    // GPT waits on the user: its question shows in the room; a settled one is a note there.
    const listed = await m.listThreads();
    const asks = L.roomAsks(i.participants, listed.threads);
    expect(asks.map((a) => [a.participant.slug, a.text])).toEqual([["gpt", "GPT wants to write apps/client/src/conn.ts"]]);
    expect(L.roomNote((t.entries.at(-1) as { text: string }).text, i.participants)).toMatchObject({ kind: "asked", outcome: "declined", slug: "gpt" });
    const u = await read(L.roomUpdate(ROOM, { mute: ["gpt"], mode: "parallel", hopCap: 1, add: [L.modelSpec({ model: "anthropic/claude-sonnet-5-5" })] }));
    expect(u.participants.map((p) => [p.slug, p.muted])).toEqual([["opus", false], ["gemini", false], ["gpt", true], ["sonnet", false]]);
    expect([u.mode, u.hop_cap]).toEqual(["parallel", 1]);
    const rows = await m.listThreads();
    expect(rows.threads.find((x) => x.id === ROOM)?.room_mode).toBe("parallel");
    expect(rows.threads.filter((x) => x.parent === ROOM)).toHaveLength(3);
    await read(L.roomUpdate(ROOM, { remove: ["sonnet"], mode: "sequential", unmute: ["gpt"] }));
    const sent = await m.call({ Send: { thread: ROOM, text: "@gpt short answer", from: null, keep: false, interject: true, scopes: [] } });
    expect(sent).toEqual({ Lines: ["a round runs on it"] });
    const again = await m.call({ Send: { thread: ROOM, text: "and one more", from: null, keep: false, interject: true, scopes: [] } });
    expect(again).toEqual({ Lines: ["taken by the round running in the room"] });
    await m.call({ Cancel: { thread: ROOM } });
    const made = await read(L.roomCreate({ from: "2026-11-02T0951Z-gamer-a2", models: [{ model: "google/gemini-3-pro" }] }));
    expect(made.participants.map((p) => p.slug)).toEqual(["opus", "gemini"]);
    expect(made.promoted_from).toBe("2026-11-02T0951Z-gamer-a2");
    const seed = (await m.transcript(made.room)).entries[0]!;
    expect(seed.kind === "user" && seed.from).toBe("room:seed");
  });
});
