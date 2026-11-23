// The map as a view of the client (design §2.25, "The map"): the toolbar, the
// canvas (Islands), the legend, the detail card. Mounted the first time it is
// shown and kept mounted, hidden, while a conversation is open: its canvas,
// transform, focus and layout stay as they were, so coming back restores it
// instantly and exactly.
import { effect } from "@preact/signals";
import { useEffect, useRef, useState } from "preact/hooks";
import * as S from "../store";
import { homeMode, mapDim, setHomeMode, setMapDim } from "../nav";
import { Card } from "./Card";
import { ScopeTray } from "./ScopeTray";
import { createIslands, type Islands } from "./islands";
import { TIMES } from "./model";
import * as M from "./state";
import { sideCollapsed, toggleSide } from "../sidebar/width";
import { useCanvasMeasure } from "./text";
import { devParams, devSpace } from "./dev";
import { loadSpace, webgl2 } from "./space/load";
import type { Space, View2D } from "./space/types";
import "./space/space.css";

const INSTANT = typeof location !== "undefined" && /instant/.test(location.search);

/** The Map | List switch of the home view. */
export function HomeSwitch() {
  const mode = homeMode.value;
  return (
    <div class="seg homeswitch" role="group" aria-label="Home view">
      <button class={mode === "map" ? "on" : ""} onClick={() => setHomeMode("map")}>Map</button>
      <button class={mode === "list" ? "on" : ""} onClick={() => setHomeMode("list")}>List</button>
    </div>
  );
}

/** The map's 2D | 3D switch (Islands | Space). */
function DimSwitch() {
  const d = mapDim.value;
  return (
    <div class="seg dimswitch" role="group" aria-label="Map view">
      <button class={d === "2d" ? "on" : ""} title="Islands: the map in 2D" onClick={() => setMapDim("2d")}>2D</button>
      <button class={d === "3d" ? "on" : ""} title="Space: the map in 3D, height = recency" onClick={() => setMapDim("3d")}>3D</button>
    </div>
  );
}

type SpaceState = { k: "idle" } | { k: "loading" } | { k: "ready" } | { k: "failed"; why: string };

export function MapView({ on }: { on: boolean }) {
  const onRef = useRef(on);
  onRef.current = on;
  const canvas = useRef<HTMLCanvasElement>(null);
  const host = useRef<HTMLDivElement>(null);
  const eng = useRef<Islands | null>(null);
  const qbox = useRef<HTMLInputElement>(null);
  const spaceHost = useRef<HTMLDivElement>(null);
  const alive = useRef(true);
  const space = useRef<Space | null>(null);
  const [sp, setSp] = useState<SpaceState>({ k: "idle" });
  const dim = mapDim.value;
  /** The mode last on screen, and what 2D looked at when 3D was asked for (undefined: nothing to hand over). */
  const shownDim = useRef<"2d" | "3d" | null>(null);
  const handView = useRef<View2D | null | undefined>(undefined);

  useEffect(() => {
    useCanvasMeasure();
    const e = createIslands(canvas.current!, host.current!, {
      model: () => M.model.peek(),
      stats: () => M.stats.peek(),
      filter: () => M.filter.peek(),
      sel: () => M.sel.peek(),
      opened: () => (S.homeView.peek() ? null : M.openedId()),
      select: (s) => M.select(s, "islands"),
      hover: (h) => (M.hover.value = h),
      open: M.openSel,
      picked: () => M.pickedIds.peek(),
      togglePick: M.togglePick,
    });
    eng.current = e;
    const stops = [
      // Islands first: Space stands on its packing.
      effect(() => { const c = M.change.value; if (M.loaded.peek()) { e.update(c.kind); space.current?.update(c.kind); } }),
      effect(() => { void M.sel.value; e.selected(M.selFrom.v); space.current?.selected(M.selFrom.v); }),
      effect(() => { void M.hover.value; void S.selected.value; void S.homeView.value; void M.picked.value; e.redraw(); space.current?.redraw(); }),
    ];
    devParams(e);
    return () => { alive.current = false; stops.forEach((f) => f()); e.destroy(); space.current?.destroy(); space.current = null; };
  }, []);

  // Space is loaded the first time 3D is shown (a separate chunk with three.js); a failure leaves 2D as it was.
  const want3d = on && dim === "3d";
  useEffect(() => {
    if (!want3d || sp.k !== "idle") return;
    if (!webgl2()) { setSp({ k: "failed", why: "this browser gives no WebGL 2 here" }); return; }
    setSp({ k: "loading" });
    loadSpace().then((mod) => {
      if (!alive.current || !spaceHost.current || space.current) return;
      const s = mod.createSpace(spaceHost.current, {
        model: () => M.model.peek(),
        stats: () => M.stats.peek(),
        filter: () => M.filter.peek(),
        sel: () => M.sel.peek(),
        hovered: () => M.hover.peek(),
        opened: () => (S.homeView.peek() ? null : M.openedId()),
        layout: () => eng.current?.layout() ?? null,
        select: (x) => M.select(x, "space"),
        hover: (h) => (M.hover.value = h),
        open: M.openSel,
        picked: () => M.pickedIds.peek(),
        togglePick: M.togglePick,
        instant: INSTANT,
        cardWidth: () => ((spaceHost.current?.clientWidth ?? 0) > 700 ? 340 : 0),
      });
      space.current = s;
      if (M.loaded.peek()) s.update("data");
      devSpace(s);
      setSp({ k: "ready" });
    }).catch((err: unknown) => {
      if (alive.current) setSp({ k: "failed", why: err instanceof Error ? err.message : String(err) });
    });
  }, [want3d, sp.k]);

  // After a failure, going back to 2D lets the next 3D try again (the chunk may load now).
  useEffect(() => { if (dim === "2d" && sp.k === "failed") setSp({ k: "idle" }); }, [dim]);

  // Which engine is on screen. Switching keeps the selection and the place: 2D → 3D starts above what Islands showed;
  // 3D → 2D zooms Islands to the selected effort, or to the place under the camera. Opening a conversation and coming
  // back keeps both engines as they were (hidden, not rendering).
  useEffect(() => {
    const e = eng.current, s = space.current;
    if (!on) { e?.hide(); s?.hide(); return; }
    M.shown();
    const was = shownDim.current;
    if (dim === "3d") {
      if (was === "2d") handView.current = e?.view() ?? null;
      e?.hide();
      if (s) { s.show(handView.current); handView.current = undefined; shownDim.current = "3d"; }
      else if (sp.k === "failed") { e?.show(); }
    } else {
      s?.hide();
      e?.show();
      if (was === "3d" && s) e?.handover(M.sel.peek(), s.target());
      shownDim.current = "2d";
      handView.current = undefined;
    }
    if (dim === "2d" || s) shownDim.current = dim;
  }, [on, dim, sp.k]);

  // Keys that belong to the map: `/` searches, Esc clears the search or the selection.
  useEffect(() => {
    const k = (ev: KeyboardEvent) => {
      if (!onRef.current) return;
      const inField = (ev.target as HTMLElement | null)?.closest?.("input, textarea, select, [contenteditable]");
      if (ev.key === "/" && !inField) { ev.preventDefault(); qbox.current?.focus(); }
      else if (ev.key === "Escape") {
        if (document.activeElement === qbox.current) qbox.current?.blur();
        else if (M.sel.peek()) M.select(null, "ui");
      }
    };
    document.addEventListener("keydown", k);
    return () => document.removeEventListener("keydown", k);
  }, []);

  const f = M.filter.value;
  const st = M.stats.value;
  const m = M.model.value;
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const real = m.real, effortN = st.nEffortsVis;
  const statLine = `${real} threads · ${effortN} efforts · ${st.nPlacesVis !== m.places.length ? `${st.nPlacesVis} of ${m.places.length}` : m.places.length} places` +
    (m.orphans ? ` · ${m.orphans} with parent not loaded` : "") + (st.query ? ` · ${st.hits} match` : "");
  const err = M.failure.value;
  const in3d = dim === "3d" && sp.k !== "failed";
  const first = !M.loaded.value;
  return (
    <main class="mapmain" hidden={!on}>
      <header class="maphead">
        {sideCollapsed.value && <button class="expand" title="Show the sidebar" aria-label="Show the sidebar" onClick={toggleSide}>☰</button>}
        <HomeSwitch />
        <DimSwitch />
        <input ref={qbox} class="mapq" type="search" placeholder="Search threads…" autocomplete="off" spellcheck={false}
          onInput={(e) => { const v = e.currentTarget.value; clearTimeout(timer.current); timer.current = setTimeout(() => M.setFilter({ q: v }), 110); }}
          onKeyDown={(e) => {
            if (e.key !== "Enter" || !f.q) return;
            const hit = m.efforts.filter((x) => x.fade >= 0.99).sort((a, b) => b.e - a.e)[0];
            if (hit) M.select({ kind: "effort", id: hit.id }, "search");
          }} />
        <div class="seg" role="group" aria-label="Time">
          {TIMES.map((t) => <button key={t} class={f.time === t ? "on" : ""} onClick={() => M.setFilter({ time: t })}>{t}</button>)}
        </div>
        <button class={"tog" + (f.needs ? " on" : "")} title="Only what needs you" onClick={() => M.setFilter({ needs: !f.needs })}><i class="mdot need" />needs you {st.needs ? <b>{st.needs}</b> : null}</button>
        <button class={"tog unread" + (st.unreadOn ? " on" : "")} onClick={() => M.setFilter({ unread: !st.unreadOn })}
          title={st.unreadOn ? "Showing unread marks (quiet outlines). Click to hide." : st.unreadFrac > 0.4 ? "Most threads are unread, so the marks are off. Click to show." : "Show unread marks"}>
          <i class="mdot unread" />unread {real ? <b>{Math.round(100 * st.unread / real)}%</b> : null}
        </button>
      </header>
      <div class="mapstage">
        <span class="mapstat">{M.loaded.value ? statLine : ""}</span>
        <div class="mapcanvas" ref={host} hidden={in3d}><canvas ref={canvas} aria-label="Map of conversations: places, efforts and the threads spawned in them" /></div>
        <div class="spacehost" ref={spaceHost} hidden={!in3d} />
        {in3d && sp.k === "ready" && (
          <div class="spacebtns">
            <button title="Look straight down: the same picture as 2D" onClick={() => space.current?.topDown()}>Top-down</button>
            <button title="The tilted view of everything" onClick={() => space.current?.overview()}>Overview</button>
          </div>
        )}
        {dim === "3d" && sp.k === "loading" && <div class="mapoverlay clear"><div class="spin" /></div>}
        {dim === "3d" && sp.k === "failed" && (
          <div class="spacefail" role="status">3D is unavailable here ({sp.why}). The 2D map works. <button onClick={() => setMapDim("2d")}>Use 2D</button></div>
        )}
        <div class="maplegend"><span><i class="mdot work" />working</span><span><i class="mdot need" />needs you</span><span><i class="mdot unread" />unread</span><span><i class="mdot idle" />idle · fades with age</span></div>
        <div class="maphint">{in3d
          ? "drag to orbit · right-drag or shift-drag to pan · scroll / pinch to zoom · click to select · double-click or Enter to open · height = recency"
          : "scroll / pinch to zoom · drag to pan · click an island to dive in · click empty space to go back · double-click or Enter to open"} · shift-click to pick for a chat</div>
        <Card />
        <ScopeTray />
        {(first || err) && (
          <div class="mapoverlay">
            {err ? (
              <>
                <div class="big">Could not load the threads</div>
                <div class="meta">{err}</div>
                <button onClick={() => void M.load()}>Retry</button>
              </>
            ) : (
              <>
                <div class="spin" />
                <div class="big">Loading threads…</div>
              </>
            )}
          </div>
        )}
        {!first && !err && m.places.length === 0 && <div class="mapoverlay clear"><div class="meta">No threads yet.</div></div>}
        {!first && m.places.length > 0 && st.nPlacesVis === 0 && <div class="mapoverlay clear"><div class="meta">Nothing active in this window.</div></div>}
      </div>
    </main>
  );
}
