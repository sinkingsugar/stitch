// `bun test` (in apps/client, `just client-test`): what the transcript shows of a
// message sent from here while the node takes it (src/transcript/outbox.ts).
import { describe, expect, test } from "bun:test";
import * as O from "../src/transcript/outbox";
import type { Entry } from "../src/model";

const at = (s: number) => new Date(Date.UTC(2026, 10, 3, 7, 0, s)).toISOString();
const user = (text: string, s: number, extra: Partial<O.UserEntry> = {}): O.UserEntry => ({
  n: 1, kind: "user", from: null, at: at(s), text, images: [], ...extra,
});
const local = (text: string, s: number, outgoing: O.Outgoing = "sending") => user(text, s, { n: -1, pending: true, outgoing });
const asst = (text: string, s: number): Entry => ({ n: 2, kind: "assistant", from: null, at: at(s), text, images: [] });

describe("what the node said of a Send", () => {
  test("woven, started, or queued behind the turn running", () => {
    expect(O.sentAs("sent to thread t, on gamer: woven into its running turn", true)).toBe("weaving");
    expect(O.sentAs("sent to thread t, on gamer: no turn runs on it, so it starts a turn", false)).toBe("starting");
    // A turn starting (its loop not begun) takes no interjection: the new
    // turn waits for it.
    expect(O.sentAs("sent to thread t, on gamer: no turn runs on it, so it starts a turn", true)).toBe("queued");
    expect(O.sentAs(undefined, false)).toBe("starting");
  });
});

describe("a message sent from here", () => {
  test("stays shown, after the node's transcript, until the transcript has it", () => {
    const mine = local("second", 10, "queued");
    const node: Entry[] = [user("first", 1), asst("reply", 2)];
    // Read again while its turn waits: still there, at the end.
    expect(O.merge(node, [mine])).toEqual([...node, mine]);
    // Written with its turn's first step: the node's entry stands for it.
    const written = [...node, user("second", 12)];
    expect(O.merge(written, [mine])).toEqual(written);
  });

  test("an older message of the same text is not it", () => {
    const mine = local("ok", 600, "starting");
    const node: Entry[] = [user("ok", 1)];
    expect(O.merge(node, [mine])).toEqual([...node, mine]);
  });

  test("two of the same text are matched one each", () => {
    const a = local("again", 10, "running"), b = local("again", 11, "queued");
    const node: Entry[] = [user("again", 12)];
    expect(O.merge(node, [a, b])).toEqual([...node, b]);
  });

  test("two woven into one step are written as one message: each is its part", () => {
    // The node writes messages that came in a row from one sender as one,
    // their texts a line apart: both bubbles stayed at the bottom for good.
    const a = local("look at the logs", 10, "running"), b = local("and the config\ntoo", 11, "running");
    const node: Entry[] = [user("first", 1), asst("reply", 2), user("look at the logs\nand the config\ntoo", 12)];
    expect(O.merge(node, [a, b])).toEqual(node);
    // A third of the same text is not in it.
    const c = local("look at the logs", 13, "weaving");
    expect(O.merge(node, [a, b, c])).toEqual([...node, c]);
    // Nor is a line of a message that is not a run of whole lines.
    expect(O.merge([user("look at the logs please", 12)], [a])).toHaveLength(2);
  });

  test("woven before the node's answer came: the note is ours", () => {
    const [read, ours] = O.wove([local("b", 2, "sending")], "b");
    expect(ours).toBe(true);
    expect(O.locals(read).map((e) => e.outgoing)).toEqual(["running"]);
  });

  test("a failed one stays, with why", () => {
    let list: Entry[] = [user("first", 1), local("lost", 5)];
    list = O.mark(list, at(5), "failed", "busy: 8 requests running");
    const e = O.locals(list)[0]!;
    expect(O.said(e)).toBe("not sent: busy: 8 requests running");
    expect(O.merge([user("first", 1), user("lost", 6)], O.locals(list))).toHaveLength(3);
  });

  test("its turn starting, a woven message read, a turn ending", () => {
    let list: Entry[] = [local("a", 1, "starting"), local("b", 2, "weaving")];
    expect(O.awaiting(list)).toBe(true);
    list = O.started(list);
    expect(O.locals(list).map((e) => e.outgoing)).toEqual(["running", "weaving"]);
    expect(O.awaiting(list)).toBe(false);
    const [read, ours] = O.wove(list, "b\n");
    expect(ours).toBe(true);
    expect(O.locals(read).map((e) => e.outgoing)).toEqual(["running", "running"]);
    expect(O.wove(list, "someone else's note")[1]).toBe(false);
    // Not read before the turn ended: the node starts a turn for it.
    expect(O.locals(O.ended(list)).map((e) => e.outgoing)).toEqual(["running", "starting"]);
  });

  test("each state says where it stands", () => {
    expect(O.said(local("x", 1, "sending"))).toBe("sending…");
    expect(O.said(local("x", 1, "queued"))).toContain("queued");
    expect(O.said(local("x", 1, "weaving"))).toContain("next step");
    expect(O.said(local("x", 1, "running"))).toBeNull();
  });
});
