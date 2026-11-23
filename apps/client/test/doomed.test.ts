// `bun test` (in apps/client): the delete dialog says what goes and why
// (design §2.5, "Deleting a thread").
import { expect, test } from "bun:test";
import { explain, label, parse } from "../src/actions/doomed";

test("lines parse with and without a role", () => {
  expect(parse("a\tTalk\tearlier")).toEqual({ id: "a", title: "Talk", role: "earlier" });
  expect(parse("a\tTalk")).toEqual({ id: "a", title: "Talk", role: null });
  expect(parse("a\tA\tB")).toEqual({ id: "a", title: "A\tB", role: null });
  expect(parse("a")).toEqual({ id: "a", title: "", role: null });
  expect(label(parse("a\tT\tlater"))).toBe("later part");
});

test("one thread: no list, no talk of parts", () => {
  const e = explain([parse("a\tTalk\tthis")]);
  expect(e.title).toBe("Delete “Talk” for good?");
  expect(e.detail).toBe("On every node. The threads it started and the forks made of it stay.");
  expect(e.showList).toBe(false);
});

test("a compacted conversation: its parts are said and listed", () => {
  const e = explain(["a\tTalk\tearlier", "b\tTalk\tearlier", "c\tTalk\tthis"].map(parse));
  expect(e.title).toBe("Delete “Talk” for good?");
  expect(e.detail).toContain("summarized and continued in a new thread twice when it grew long, so it is 3 parts of one conversation: all 3 go.");
  expect(e.showList).toBe(true);
});

test("an older node, without roles", () => {
  const e = explain(["a\tTalk", "b\tTalk"].map(parse));
  expect(e.title).toBe("Delete 2 threads for good?");
  expect(e.detail).toContain("with what they were summarized from or into");
});
