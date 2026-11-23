import { expect, test } from "bun:test";
import { apply, merge, withCall, type ListState } from "../src/deltas";
import type { ThreadItem } from "../src/model";

const row = (id: string, last: string, o: Partial<ThreadItem> = {}): ThreadItem =>
  ({ id, title: id, place: "global", pinned: false, last_step: last, state: "idle", in_flight: null, earlier: [], ...o }) as unknown as ThreadItem;

const base = (): ListState => ({
  v: 1,
  threads: [row("a", "2026-11-15T10:00:00Z"), row("b", "2026-11-15T09:00:00Z"), row("c", "2026-11-15T08:00:00Z")],
  places: [{ place: "global", count: 3 }] as never,
  placeRows: { stitch: [row("s1", "2026-11-14T10:00:00Z", { place: "stitch" })] },
  extra: { old: row("old", "2026-10-01T00:00:00Z") },
});

test("a row that moved is put in place and the list ordered again", () => {
  const s = apply(base(), { v: 2, upsert: [row("c", "2026-11-15T11:00:00Z", { title: "C" })] }, 300);
  if (s === "resync") throw new Error("resync");
  expect(s.v).toBe(2);
  expect(s.threads.map((t) => t.id)).toEqual(["c", "a", "b"]);
  expect(s.threads[0].title).toBe("C");
});

test("removed threads and those compacted into a row leave the list", () => {
  const s = apply(base(), { v: 2, upsert: [row("d", "2026-11-15T12:00:00Z", { earlier: ["a"] })], remove: ["b"] }, 300);
  if (s === "resync") throw new Error("resync");
  expect(s.threads.map((t) => t.id)).toEqual(["d", "c"]);
});

test("a frame out of order asks for the list again; a reset replaces it", () => {
  expect(apply(base(), { v: 3, upsert: [] }, 300)).toBe("resync");
  const s = apply(base(), { v: 1, reset: { threads: [row("x", "2026-11-15T00:00:00Z")], places: [] } }, 300);
  if (s === "resync") throw new Error("resync");
  expect(s.v).toBe(1);
  expect(s.threads.map((t) => t.id)).toEqual(["x"]);
  expect(s.places).toEqual([]);
});

test("a new row enters the window only when it falls in it", () => {
  const rows = base().threads;
  expect(merge(rows, [row("old2", "2026-01-01T00:00:00Z")], [], 3).map((t) => t.id)).toEqual(["a", "b", "c"]);
  expect(merge(rows, [row("new", "2026-11-16T00:00:00Z")], [], 3).map((t) => t.id)).toEqual(["new", "a", "b"]);
  expect(merge(rows, [row("pin", "2026-01-01T00:00:00Z", { pinned: true })], [], 3)[0].id).toBe("pin");
});

test("place sections follow their place; the open thread's row follows too", () => {
  const s = apply(
    base(),
    {
      v: 2,
      upsert: [
        row("s2", "2026-11-15T13:00:00Z", { place: "stitch" }),
        row("s1", "2026-11-15T13:30:00Z", { place: "otter" }),
        row("old", "2026-11-15T14:00:00Z", { title: "Old, renamed" }),
      ],
    },
    300,
  );
  if (s === "resync") throw new Error("resync");
  expect(s.placeRows.stitch.map((t) => t.id)).toEqual(["s2"]);
  expect(s.extra.old?.title).toBe("Old, renamed");
  const gone = apply(base(), { v: 2, remove: ["old"] }, 300);
  if (gone === "resync") throw new Error("resync");
  expect(gone.extra.old).toBeNull();
});

test("a running call changes that row's in_flight alone", () => {
  const rows = base().threads;
  const f = { name: "shell_run", headline: "$ ls", summary: "ls", since: null };
  const out = withCall(rows, "b", f);
  expect(out[1].in_flight).toEqual(f);
  expect(out[0]).toBe(rows[0]);
  expect(withCall(out, "b", f)).toBe(out);
  expect(withCall(rows, "nope", f)).toBe(rows);
});
