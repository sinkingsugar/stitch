// `bun test`: where a thread's action buttons show, which set is live and
// how each button's latest press ended (design §2.30, step 2;
// src/transcript/suggested.ts).
import { describe, expect, test } from "bun:test";
import { actionBlocks, replyBlocks } from "../src/transcript/suggested";
import type { Entry } from "../src/model";
import type { ActionButton } from "../src/payload";

const at = "2026-11-20T10:00:00Z";
const user = (text: string): Entry => ({ n: 1, kind: "user", from: null, at, text, images: [] });
const asst = (text: string): Entry => ({ n: 2, kind: "assistant", from: null, at, text, images: [] });
const button = (label: string): ActionButton => ({ label, tool: "otter_run", call: `otter_run\nenter("${label}")`, wake: false });
const offer = (set: string, labels: string[], ok = true): Entry => ({
  n: 3, kind: "tool", at, name: "offer_actions", call: "offer_actions: …", result: { ok, output: ok ? "Shown." : "no tool" },
  ...(ok ? { set, actions: labels.map(button) } : {}),
});
const press = (set: string, item: number, ok: boolean): Entry => ({
  n: 4, kind: "press", at, text: `▶ x: ${ok ? "ok" : "error"}`, set, item, label: "x", tool: "otter_run", ok, output: "", wake: false,
});

describe("action buttons", () => {
  test("the latest set is live at its turn's end, and stays so after the user writes", () => {
    const es = [user("waypoints?"), offer("o1", ["Dock", "Ring", "Gate"]), asst("Pick one."), user("thanks"), asst("Sure.")];
    const b = actionBlocks(es);
    expect([...b.keys()]).toEqual([2]);
    expect(b.get(2)?.live).toBe(true);
    expect(b.get(2)?.actions.map((a) => a.label)).toEqual(["Dock", "Ring", "Gate"]);
    // A turn running changes nothing: presses wait for it on the node.
    expect(actionBlocks([...es, user("more")]).get(2)?.live).toBe(true);
  });

  test("each button shows its latest press; presses do not move the set", () => {
    const es = [offer("o1", ["Dock", "Ring"]), asst("Pick."), press("o1", 0, false), press("o1", 0, true), press("o1", 1, false)];
    const b = actionBlocks(es).get(1)!;
    expect(b.last).toEqual([true, false]);
    expect(b.live).toBe(true);
  });

  test("a newer set replaces the older, which stays inert where it was", () => {
    const es = [offer("o1", ["A"]), asst("One."), user("again"), offer("o2", ["B"]), asst("Two."), press("o1", 0, true)];
    const b = actionBlocks(es);
    expect([...b.keys()]).toEqual([1, 4]);
    expect(b.get(1)).toMatchObject({ set: "o1", live: false, last: [true] });
    expect(b.get(4)).toMatchObject({ set: "o2", live: true, last: [null] });
  });

  test("a refused offer shows nothing and replaces nothing", () => {
    const es = [offer("o1", ["A"]), asst("One."), offer("o2", ["B"], false), asst("Sorry.")];
    const b = actionBlocks(es);
    expect([...b.keys()]).toEqual([3]);
    expect(b.get(3)?.set).toBe("o1");
    expect(b.get(3)?.live).toBe(true);
  });

  test("a press that woke the agent is the user's message: it ends preset answers, not the set", () => {
    const suggest: Entry = { n: 5, kind: "tool", at, name: "suggest_replies", call: "", result: { ok: true, output: "" }, replies: [{ label: "Yes", text: "Yes" }] };
    const es = [offer("o1", ["Dock"]), suggest, asst("Dock, or yes?"), { ...press("o1", 0, true), wake: true }, user("▶ Dock: ok (otter_run)"), asst("Docked.")];
    expect(replyBlocks(es, false).size).toBe(0);
    expect(actionBlocks(es).get(2)?.live).toBe(true);
  });
});
