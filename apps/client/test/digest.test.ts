import { describe, expect, test } from "bun:test";
import { asksTitle, counts, grouped, inConversation, KIND_ORDER, said } from "../src/digest/logic";
import type { DigestItem, DigestKind } from "../src/payload";

const item = (kind: DigestKind, text: string, thread = "t1"): DigestItem => ({ kind, text, at: `${thread}#m1`, thread, msg: "m1" });

describe("digest items", () => {
  const items = [item("decision", "d1"), item("goal", "g"), item("ask", "a1"), item("finding", "f1"), item("decision", "d2"), item("question", "q1"), item("change", "c"), item("outcome", "o"), item("ask", "a2")];

  test("are grouped in the order of what needs the user first, each group in the order given", () => {
    const g = grouped(items);
    expect(g.map((x) => x.kind)).toEqual(["ask", "question", "outcome", "decision", "finding", "change", "goal"]);
    expect(g[0]!.items.map((i) => i.text)).toEqual(["a1", "a2"]);
    expect(g[3]!.items.map((i) => i.text)).toEqual(["d1", "d2"]);
    expect(KIND_ORDER).toHaveLength(7);
  });

  test("leave out the kinds they have none of", () => {
    expect(grouped([item("goal", "g")]).map((x) => x.kind)).toEqual(["goal"]);
    expect(grouped([])).toEqual([]);
  });

  test("are counted in a line, asks and open first, the goal not at all", () => {
    expect(counts(items)).toBe("2 asks · 1 open · 2 decisions · 1 finding · 1 change · 1 outcome");
    expect(counts([item("decision", "a"), item("decision", "b"), item("decision", "c"), item("question", "q"), item("question", "r"), item("ask", "x")])).toBe("1 ask · 2 open · 3 decisions");
    expect(counts([item("goal", "g")])).toBe("");
  });

  test("open their message in the open conversation, or in another thread first", () => {
    const row = { id: "t3", earlier: ["t1", "t2"] };
    expect(inConversation(item("goal", "g", "t3"), row)).toBe(true);
    expect(inConversation(item("goal", "g", "t1"), row)).toBe(true);
    expect(inConversation(item("goal", "g", "t9"), row)).toBe(false);
    expect(inConversation(item("goal", "g", "t1"), null)).toBe(false);
  });
});

describe("what an update says", () => {
  const u = { why: null, usd: 0.0013, model: "m", windows: 1, more: false, dropped: 0, day_usd: 0.3, cap_usd: 2 };
  test("a written one gives its cost", () => expect(said({ ...u, outcome: "written" })).toBe("Updated ($0.0013)."));
  test("several windows and more to come are told", () => expect(said({ ...u, outcome: "written", windows: 3, more: true })).toBe("Updated ($0.0013, 3 windows, more to read later)."));
  test("the cap is told with the day's total", () => expect(said({ ...u, outcome: "capped", day_usd: 2 })).toBe("Today's cap is reached ($2.00 of $2.00): no update."));
  test("a failure says why", () => expect(said({ ...u, outcome: "failed", why: "unreadable reply" })).toBe("The update failed: unreadable reply."));
  test("nothing new and busy", () => {
    expect(said({ ...u, outcome: "nothing new" })).toBe("Nothing new to digest.");
    expect(said({ ...u, outcome: "busy" })).toBe("An update is already running.");
  });
  test("the asks mark's tooltip counts", () => {
    expect(asksTitle(1)).toBe("asks you 1 thing");
    expect(asksTitle(2)).toBe("asks you 2 things");
  });
});
