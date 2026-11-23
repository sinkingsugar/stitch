// Mock-only URL parameters for screenshots (`?mock#/map&…` is not a thing: they are query params):
//   mapsel=place:<key> | effort:<id|bigN> | thread:<id|needN>   select it (the map flies there)
//   maptime=today|3d|week|month|all   mapq=<text>   mapneeds   instant (no animation)
//   maptop   Space (`#/map/3d`) looks straight down
import { effect } from "@preact/signals";
import type { Islands } from "./islands";
import type { Space } from "./space/types";
import * as M from "./state";
import type { Sel } from "./model";

export function devParams(e: Islands) {
  const q = new URLSearchParams(location.search);
  if (!q.has("mock")) return;
  (window as unknown as { __atlas: unknown }).__atlas = { ...e.debug, sel: () => M.sel.value, filter: () => M.filter.value };
  const stop = effect(() => {
    if (!M.loaded.value) return;
    queueMicrotask(() => stop());
    setTimeout(() => {
      const patch: Record<string, unknown> = {};
      const t = q.get("maptime"); if (t) patch.time = t;
      const s = q.get("mapq"); if (s) patch.q = s;
      if (q.has("mapneeds")) patch.needs = true;
      if (Object.keys(patch).length) M.setFilter(patch);
      const sel = q.get("mapsel");
      if (sel) {
        const [kind, ...rest] = sel.split(":");
        let id = rest.join(":");
        // `effort:big0` the largest effort, `big1` the next…; `thread:need0` the first thread that needs you.
        const m = M.model.value;
        const big = /^big(\d+)$/.exec(id), need = /^need(\d+)$/.exec(id);
        if (big) id = m.efforts.slice().sort((a, b) => b.threads.length - a.threads.length)[+big[1]!]?.id ?? id;
        if (need) id = m.threads.filter((x) => x.need)[+need[1]!]?.id ?? id;
        M.select({ kind: kind as Sel["kind"], id }, "test");
      }
    }, 50);
  });
}

/** Space's mock-only hooks: `window.__space` (draw calls, instances, frame time, camera, labels, a pick) and `maptop`. */
export function devSpace(s: Space) {
  const q = new URLSearchParams(location.search);
  if (!q.has("mock")) return;
  (window as unknown as { __space: unknown }).__space = s.debug;
  if (!q.has("maptop")) return;
  const stop = effect(() => {
    if (!M.loaded.value) return;
    queueMicrotask(() => stop());
    setTimeout(() => s.topDown(), 120);
  });
}
