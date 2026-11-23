// `bun test`: the history behind opening from the map and coming back (src/nav.ts), on a fake browser history.
import { expect, test } from "bun:test";

const kept = new Map<string, string>();
(globalThis as any).localStorage = { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v), removeItem: (k: string) => void kept.delete(k) };
// A history of entries {url, state}, with go(): what the browser does.
const entries: { hash: string; state: unknown }[] = [{ hash: "#/map", state: null }];
let at = 0;
const loc = { get hash() { return entries[at]!.hash; } };
const listeners: (() => void)[] = [];
(globalThis as any).location = loc;
(globalThis as any).history = {
  get state() { return entries[at]!.state; },
  pushState(state: unknown, _t: string, url: string) { entries.length = at + 1; entries.push({ hash: url, state }); at++; },
  replaceState(state: unknown, _t: string, url: string) { entries[at] = { hash: url, state }; },
  go(n: number) { at += n; listeners.forEach((f) => f()); },
};
(globalThis as any).window = { addEventListener: (_: string, f: () => void) => listeners.push(f) };

const Nav = await import("../src/nav");
const { homeView } = await import("../src/home/source");

test("opening from the map, through a chain of threads, and back restores the map's entry", () => {
  homeView.value = true;
  const applied: string[] = [];
  Nav.start((r) => applied.push(r.kind));
  expect(Nav.homeMode.value).toBe("map"); // `#/map` says so, and it is remembered
  expect(kept.get("stitch.homeMode")).toBe("map");
  expect(Nav.mapShown.value).toBe(true);

  Nav.opening("a", "push");
  homeView.value = false;
  expect(entries[at]!.hash).toBe("#/t/a?from=map");
  expect(Nav.fromMap.value).toBe(true);
  expect(Nav.depth.value).toBe(1);

  Nav.opening("b", "push"); // a thread opened from that conversation (the spawn panel): the chain goes on
  expect(entries[at]!.hash).toBe("#/t/b?from=map");
  expect(Nav.depth.value).toBe(2);

  Nav.opening("b2", "replace"); // compaction: the same place in the chain
  expect(entries[at]!.hash).toBe("#/t/b2?from=map");
  expect(Nav.depth.value).toBe(2);
  expect(entries.length).toBe(3);

  Nav.back(); // two steps: the map's own entry
  expect(at).toBe(0);
  expect(Nav.route.value).toEqual({ kind: "map" });
  expect(Nav.fromMap.value).toBe(false);
  expect(applied.at(-1)).toBe("map");
  // The browser's forward lands on the first conversation, still from the map.
  history.go(1);
  expect(Nav.route.value).toEqual({ kind: "thread", id: "a", from: "map" });
  expect(Nav.depth.value).toBe(1);
});

test("a conversation opened from the list has no way back to the map; the switch is remembered", () => {
  Nav.setHomeMode("list");
  homeView.value = true;
  expect(kept.get("stitch.homeMode")).toBe("list");
  Nav.homing("push");
  expect(entries[at]!.hash).toBe("#/");
  Nav.opening("c", "push");
  expect(entries[at]!.hash).toBe("#/t/c");
  expect(Nav.fromMap.value).toBe(false);
  // Back with no map entry in history: replaced by the map route.
  homeView.value = false;
  Nav.back();
  expect(entries[at]!.hash).toBe("#/map");
});

test("the map's 2D | 3D: remembered, in the URL, and the way back from a conversation returns to it", () => {
  homeView.value = true;
  Nav.setHomeMode("map");
  Nav.homing("push");
  expect(entries[at]!.hash).toBe("#/map");
  Nav.setMapDim("3d");
  expect(kept.get("stitch.mapView")).toBe("3d");
  expect(entries[at]!.hash).toBe("#/map/3d"); // replaced, not a new entry
  const mapAt = at;
  Nav.opening("d", "push");
  homeView.value = false;
  expect(entries[at]!.hash).toBe("#/t/d?from=map");
  Nav.back();
  expect(at).toBe(mapAt);
  expect(Nav.route.value).toEqual({ kind: "map", view: "3d" });
  expect(Nav.mapDim.value).toBe("3d");
  // A link to the 2D map says 2D; history that lost its state replaces with the map as last shown.
  homeView.value = true;
  history.pushState(null, "", "#/map");
  listeners.forEach((f) => f());
  expect(Nav.mapDim.value).toBe("2d");
  expect(kept.get("stitch.mapView")).toBe("2d");
  Nav.setMapDim("3d");
  Nav.opening("e", "push");
  homeView.value = false;
  history.replaceState(null, "", "#/t/e"); // a reload that lost the chain
  listeners.forEach((f) => f());
  Nav.back();
  expect(entries[at]!.hash).toBe("#/map/3d");
});
