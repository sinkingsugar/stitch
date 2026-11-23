// `bun test`: where a turn's preset answers show and whether they are live
// (design §2.30; src/transcript/suggested.ts).
import { describe, expect, test } from "bun:test";
import { replyBlocks } from "../src/transcript/suggested";
import type { Entry } from "../src/model";
import type { SuggestedReply } from "../src/payload";

const at = "2026-11-18T17:00:00Z";
const user = (text: string, extra: object = {}): Entry => ({ n: 1, kind: "user", from: null, at, text, images: [], ...extra });
const asst = (text: string): Entry => ({ n: 2, kind: "assistant", from: null, at, text, images: [] });
const suggest = (...labels: string[]): Entry => ({
  n: 3, kind: "tool", at, name: "suggest_replies", call: "suggest_replies", result: { ok: true, output: "Shown." },
  replies: labels.map((l): SuggestedReply => ({ label: l, text: l === "No" ? "No, stop." : l })),
});
const shell: Entry = { n: 4, kind: "tool", at, name: "shell_run", call: "$ ls", result: { ok: true, output: "" } };

describe("preset answers", () => {
  test("the latest set is live at the end of its turn, while no turn runs", () => {
    const es = [user("go?"), suggest("Yes", "No"), asst("Shall I?")];
    const b = replyBlocks(es, false);
    expect([...b.keys()]).toEqual([2]);
    expect(b.get(2)).toEqual({ replies: (es[1] as { replies: SuggestedReply[] }).replies, live: true, picked: null });
    // A turn runs (the one it was in, or another): none shown.
    expect(replyBlocks(es, true).size).toBe(0);
  });

  test("answered: shown before the message, the pick marked by its text, not live", () => {
    const es = [suggest("Yes", "No"), asst("Shall I?"), user("No, stop."), asst("Stopped.")];
    expect([...replyBlocks(es, false)]).toEqual([[1, { replies: (es[0] as { replies: SuggestedReply[] }).replies, live: false, picked: 1 }]]);
    // Pressed just now: the message is still local (pending), the same.
    const sent = [suggest("Yes", "No"), asst("Shall I?"), user("Yes", { pending: true, outgoing: "sending" })];
    expect(replyBlocks(sent, true).get(1)?.picked).toBe(0);
  });

  test("never picked (something else typed): gone", () => {
    expect(replyBlocks([suggest("Yes", "No"), asst("Shall I?"), user("maybe later")], false).size).toBe(0);
  });

  test("a later set in the turn replaces the earlier; tool entries without replies count for nothing", () => {
    const es = [suggest("A", "B"), shell, suggest("C"), asst("Pick")];
    const b = replyBlocks(es, false);
    expect(b.size).toBe(1);
    expect(b.get(3)?.replies.map((r) => r.label)).toEqual(["C"]);
    expect(replyBlocks([shell, asst("x")], false).size).toBe(0);
  });

  test("an older turn's set stays inert after a newer turn", () => {
    const es = [suggest("Yes"), user("Yes"), asst("Done."), user("now what?"), asst("Nothing.")];
    const b = replyBlocks(es, false);
    expect([...b.keys()]).toEqual([0]);
    expect(b.get(0)?.live).toBe(false);
  });
});
