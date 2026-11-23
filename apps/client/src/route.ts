// Hash routes (design §2.24, "Routes"): no router, three places.
//   #/            home (the remembered Map | List)
//   #/map         home, the map (Islands, 2D)
//   #/map/3d      home, the map in 3D (Space)
//   #/t/<id>      a conversation; `?from=map` when it was opened from the map
// Pure: parsing, formatting, and what "back to the map" costs in history steps.

export type Route =
  | { kind: "home" }
  | { kind: "map"; view?: "3d" }
  | { kind: "thread"; id: string; from?: "map" };

export function parseRoute(hash: string): Route {
  const h = hash.replace(/^#/, "");
  const [path = "", query = ""] = h.split("?", 2);
  const parts = path.split("/").filter(Boolean);
  if (parts[0] === "map" && parts.length === 1) return { kind: "map" };
  if (parts[0] === "map" && parts.length === 2 && parts[1] === "3d") return { kind: "map", view: "3d" };
  if (parts[0] === "t" && parts.length === 2) {
    let id = parts[1]!;
    try { id = decodeURIComponent(id); } catch { /* keep it raw */ }
    if (id) return new URLSearchParams(query).get("from") === "map" ? { kind: "thread", id, from: "map" } : { kind: "thread", id };
  }
  return { kind: "home" };
}

export function formatRoute(r: Route): string {
  if (r.kind === "map") return r.view === "3d" ? "#/map/3d" : "#/map";
  if (r.kind === "thread") return "#/t/" + encodeURIComponent(r.id) + (r.from === "map" ? "?from=map" : "");
  return "#/";
}

export const sameRoute = (a: Route, b: Route) => formatRoute(a) === formatRoute(b);

/** How a conversation being opened relates to the map: opened from the map (it
 * is on screen), or from another conversation that was (the chain goes on).
 * `depth`: the history steps between the conversation and the map entry. */
export function openedFrom(prev: Route, mapShown: boolean, prevDepth: number): { from?: "map"; depth: number } {
  if (prev.kind === "thread" && prev.from === "map") return { from: "map", depth: prevDepth + 1 };
  if (prev.kind !== "thread" && mapShown) return { from: "map", depth: 1 };
  return { depth: 0 };
}

/** The route of a conversation opened now. */
export function threadRoute(id: string, prev: Route, mapShown: boolean, prevDepth: number): { route: Route; depth: number } {
  const o = openedFrom(prev, mapShown, prevDepth);
  return { route: o.from ? { kind: "thread", id, from: "map" } : { kind: "thread", id }, depth: o.depth };
}

/** What the "← Map" button does: that many steps back in history (the map's
 * entry is exactly there), or, when the history does not say (a link, a reload
 * that lost its state), a replacement by the map's route (`map`: 2D or 3D, as last shown). */
export function backToMap(r: Route, depth: number | undefined, map: Route = { kind: "map" }): { back: number } | { replace: Route } {
  if (r.kind === "thread" && r.from === "map" && depth && depth > 0) return { back: depth };
  return { replace: map };
}
