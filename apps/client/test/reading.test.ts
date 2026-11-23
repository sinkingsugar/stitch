import { describe, expect, test } from "bun:test";
import { rowCounts, unfoldByDefault } from "../src/digest/logic";

// width.ts reads localStorage and window at load: give it some.
(globalThis as any).localStorage ??= { getItem: () => null, setItem() {}, removeItem() {} };
(globalThis as any).window ??= { innerWidth: 1280 };
const { cardTop, clampWidth, defaultWidth, initialWidth } = await import("../src/sidebar/width");

describe("sidebar width", () => {
  test("clamps to 260..640", () => {
    expect(clampWidth(100)).toBe(260);
    expect(clampWidth(900)).toBe(640);
    expect(clampWidth(400.4)).toBe(400);
  });
  test("defaults wider above 1400 px", () => {
    expect(defaultWidth(1280)).toBe(320);
    expect(defaultWidth(1401)).toBe(360);
  });
  test("a stored width is clamped, rubbish ignored", () => {
    expect(initialWidth("500", 1280)).toBe(500);
    expect(initialWidth("9999", 1280)).toBe(640);
    expect(initialWidth("abc", 1600)).toBe(360);
    expect(initialWidth(null, 1000)).toBe(320);
  });
});

describe("hover card", () => {
  test("sits level with its row, kept inside the window", () => {
    expect(cardTop(100, 200, 800)).toBe(100);
    expect(cardTop(700, 200, 800)).toBe(592);
    expect(cardTop(-30, 200, 800)).toBe(8);
  });
  test("counts line from a row's digest", () => {
    expect(rowCounts({ asks: 1, questions: 2 })).toBe("1 ask · 2 open");
    expect(rowCounts({ asks: 0, questions: 0 })).toBe("");
  });
});

describe("digest block fold", () => {
  const it = (kind: any) => ({ kind, text: "x", at: "t#m1", thread: "t", msg: "m1" });
  test("unfolded for an ask or an unread conversation, else folded", () => {
    expect(unfoldByDefault([it("ask")], false)).toBe(true);
    expect(unfoldByDefault([it("decision")], true)).toBe(true);
    expect(unfoldByDefault([it("decision")], false)).toBe(false);
  });
});
