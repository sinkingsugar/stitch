// `bun test`: the hash routes and what "back to the map" costs (src/route.ts).
import { expect, test } from "bun:test";
import { backToMap, formatRoute, openedFrom, parseRoute, threadRoute, type Route } from "../src/route";

test("routes parse and format, and round-trip", () => {
  const rs: Route[] = [{ kind: "home" }, { kind: "map" }, { kind: "map", view: "3d" }, { kind: "thread", id: "2026-11-02T0950Z-gamer-a1" }, { kind: "thread", id: "x y/z#1", from: "map" }];
  for (const r of rs) expect(parseRoute(formatRoute(r))).toEqual(r);
  expect(formatRoute({ kind: "thread", id: "a", from: "map" })).toBe("#/t/a?from=map");
  expect(formatRoute({ kind: "map" })).toBe("#/map");
  expect(formatRoute({ kind: "map", view: "3d" })).toBe("#/map/3d");
  expect(parseRoute("#/map/3d")).toEqual({ kind: "map", view: "3d" });
});

test("anything else is home", () => {
  for (const h of ["", "#", "#/", "#/nope", "#/t", "#/t/", "#/map/x", "#/map/3d/x", "#/t/a/b"]) expect(parseRoute(h)).toEqual({ kind: "home" });
  expect(parseRoute("#/t/a?from=elsewhere")).toEqual({ kind: "thread", id: "a" });
  expect(parseRoute("#/t/%E0%A4%A")).toEqual({ kind: "thread", id: "%E0%A4%A" }); // a bad escape stays raw
});

test("a conversation opened from the map remembers it, and chains of them count their steps", () => {
  expect(openedFrom({ kind: "home" }, true, 0)).toEqual({ from: "map", depth: 1 });
  expect(openedFrom({ kind: "map" }, true, 0)).toEqual({ from: "map", depth: 1 });
  expect(openedFrom({ kind: "home" }, false, 0)).toEqual({ depth: 0 });
  expect(openedFrom({ kind: "thread", id: "a", from: "map" }, false, 1)).toEqual({ from: "map", depth: 2 });
  expect(openedFrom({ kind: "thread", id: "a" }, true, 0)).toEqual({ depth: 0 });
  expect(threadRoute("b", { kind: "map" }, true, 0)).toEqual({ route: { kind: "thread", id: "b", from: "map" }, depth: 1 });
});

test("back goes exactly to the map's history entry, or replaces when history does not say", () => {
  expect(backToMap({ kind: "thread", id: "b", from: "map" }, 2)).toEqual({ back: 2 });
  expect(backToMap({ kind: "thread", id: "b", from: "map" }, undefined)).toEqual({ replace: { kind: "map" } });
  expect(backToMap({ kind: "thread", id: "b" }, 3)).toEqual({ replace: { kind: "map" } });
  expect(backToMap({ kind: "thread", id: "b" }, 0, { kind: "map", view: "3d" })).toEqual({ replace: { kind: "map", view: "3d" } });
});
