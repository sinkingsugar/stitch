// Navigation (design §2.24, "Routes"): the URL hash and the history entries
// behind the store's `select` / `goHome`, the home view's Map | List choice,
// and "back to the map". Every way of opening a conversation goes through
// `S.select`, which calls `opening` here; the browser's back and forward come
// back through `start`'s listener and the store's `applyRoute`.
import { computed, signal } from "@preact/signals";
import { homeView } from "./home/source";
import { backToMap, formatRoute, parseRoute, sameRoute, threadRoute, type Route } from "./route";

export type HomeMode = "list" | "map";
const MODE_KEY = "stitch.homeMode";
const loadMode = (): HomeMode => {
  try { return localStorage.getItem(MODE_KEY) === "map" ? "map" : "list"; } catch { return "list"; }
};

/** What the home view shows: today's list of sections, or the map. Remembered in this browser. */
export const homeMode = signal<HomeMode>(loadMode());

export type MapDim = "2d" | "3d";
const DIM_KEY = "stitch.mapView";
const loadDim = (): MapDim => {
  try { return localStorage.getItem(DIM_KEY) === "3d" ? "3d" : "2d"; } catch { return "2d"; }
};
/** The map's mode: Islands (2D) or Space (3D). Remembered in this browser; the route says it too (`#/map/3d`). */
export const mapDim = signal<MapDim>(loadDim());
const keepDim = (d: MapDim) => { try { localStorage.setItem(DIM_KEY, d); } catch { /* private mode */ } };
/** The map's route as it stands (2D or 3D). */
export const mapRoute = (): Route => (mapDim.value === "3d" ? { kind: "map", view: "3d" } : { kind: "map" });
const homeRoute = (): Route => (homeMode.value === "map" ? mapRoute() : { kind: "home" });
/** The route as the URL says it now. */
export const route = signal<Route>({ kind: "home" });
/** History steps between the open conversation and the map entry it came from. */
export const depth = signal(0);
/** The map is what the main area shows. */
export const mapShown = computed(() => homeView.value && homeMode.value === "map");
/** The open conversation was opened from the map: it has a "← Map" button. */
export const fromMap = computed(() => route.value.kind === "thread" && route.value.from === "map");

let apply: (r: Route) => void = () => {};

function write(r: Route, d: number, how: "push" | "replace") {
  const prev = route.value;
  route.value = r;
  depth.value = d;
  if (typeof history === "undefined" || typeof location === "undefined") return;
  const url = formatRoute(r);
  if (how === "replace") history.replaceState({ d }, "", url);
  else if (!sameRoute(prev, r) || location.hash !== url) history.pushState({ d }, "", url);
}

export function setHomeMode(m: HomeMode) {
  homeMode.value = m;
  try { localStorage.setItem(MODE_KEY, m); } catch { /* private mode */ }
  if (homeView.value) write(homeRoute(), 0, "replace");
}

/** The map's 2D | 3D switch: remembered, and the URL follows while the map is shown. */
export function setMapDim(d: MapDim) {
  mapDim.value = d;
  keepDim(d);
  if (mapShown.value) write(mapRoute(), 0, "replace");
}

/** A conversation is being opened (before the view changes): its route. `none`: the URL already says it. */
export function opening(id: string, how: "push" | "replace" | "none") {
  if (how === "none") return;
  const prev = route.value;
  if (how === "replace" && prev.kind === "thread") {
    write({ kind: "thread", id, ...(prev.from ? { from: prev.from } : {}) }, depth.value, "replace");
    return;
  }
  const t = threadRoute(id, prev, mapShown.value, depth.value);
  write(t.route, t.depth, how);
}

/** The home view is being shown (before the view changes). */
export function homing(how: "push" | "replace" | "none") {
  if (how === "none") return;
  write(homeRoute(), 0, how);
}

/** The "← Map" button, and Esc in a conversation opened from the map. */
export function back() {
  const plan = backToMap(route.value, depth.value, mapRoute());
  if ("back" in plan) history.go(-plan.back);
  else {
    write(plan.replace, 0, "replace");
    apply(plan.replace);
  }
}

/** Reads the URL once, then follows the browser's back and forward. */
export function start(applyRoute: (r: Route) => void) {
  apply = applyRoute;
  const read = () => {
    const r = parseRoute(location.hash);
    route.value = r;
    depth.value = (history.state as { d?: number } | null)?.d ?? 0;
    if (r.kind === "map") {
      homeMode.value = "map";
      try { localStorage.setItem(MODE_KEY, "map"); } catch { /* */ }
      const d: MapDim = r.view === "3d" ? "3d" : "2d";
      if (mapDim.value !== d) { mapDim.value = d; keepDim(d); }
    }
    applyRoute(r);
  };
  read();
  window.addEventListener("popstate", read);
}
