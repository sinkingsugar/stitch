import { signal } from "@preact/signals";
import * as S from "../store";
import * as H from "../home/state";
import { dotOf, money, short, titleOf, useTick, now } from "../home/util";
import { SearchBox } from "../actions/Search";
import { useEffect, useState } from "preact/hooks";
import { group, openByDefault, type Section } from "./groups";
import { asksTitle, rowCounts } from "../digest/logic";
import { ago } from "../home/util";
import { cardTop, resetSideWidth, setSideWidth, sideWidth, toggleSide } from "./width";
import { mapShown } from "../nav";
import type { ThreadItem } from "../model";

/** The card beside the sidebar: a row hovered for 400 ms or focused by keyboard. */
const card = signal<{ t: ThreadItem; top: number; left: number } | null>(null);
let showTimer: ReturnType<typeof setTimeout> | undefined;
let hideTimer: ReturnType<typeof setTimeout> | undefined;
const hideNow = () => { clearTimeout(showTimer); clearTimeout(hideTimer); card.value = null; };
function reveal(t: ThreadItem, el: HTMLElement) {
  const side = el.closest("aside")?.getBoundingClientRect();
  if (!side || window.innerWidth <= 720) return;
  card.value = { t, top: el.getBoundingClientRect().top, left: side.right + 8 };
}
/** Pointer on a row: the card follows after 400 ms, or at once when one is already up (moving between rows). */
function enter(t: ThreadItem, e: PointerEvent) {
  if (e.pointerType === "touch" || e.pointerType === "pen") return;
  const el = e.currentTarget as HTMLElement;
  clearTimeout(hideTimer); clearTimeout(showTimer);
  if (card.value) reveal(t, el);
  else showTimer = setTimeout(() => reveal(t, el), 400);
}
function leave() {
  clearTimeout(showTimer); clearTimeout(hideTimer);
  hideTimer = setTimeout(() => (card.value = null), 120);
}
function focused(t: ThreadItem, e: FocusEvent) {
  const el = e.currentTarget as HTMLElement;
  if (el.matches(":focus-visible")) reveal(t, el);
}
/** Mock-only (`?mock&hover=<id>`): shows a row's card, for screenshots. */
export function peek(id: string) {
  const el = document.querySelector<HTMLElement>(`[data-tid="${id}"]`);
  const t = el && [...S.threads.value, ...Object.values(S.placeRows.value).flat()].find((x) => x.id === id);
  if (el && t) reveal(t, el);
}

function Card() {
  const c = card.value;
  if (!c) return null;
  const { t } = c;
  const d = t.digest;
  const line = d ? rowCounts(d) : "";
  const title = t.detached ? titleOf(t) : t.title;
  const goal = (t.goal ?? "").trim();
  return (
    <div class="hcard" role="tooltip" style={`left:${c.left}px;top:${cardTop(c.top, 260, window.innerHeight)}px`}>
      <div class="hc-title">{title}</div>
      {d && <div class={"hc-head" + (d.fresh ? "" : " stale")}>{d.headline}</div>}
      {d && !d.fresh && <div class="hc-stale">stale: the conversation moved since this digest</div>}
      {line && <div class="hc-counts">{line}</div>}
      {goal && goal !== title && <div class="hc-goal">{goal}</div>}
      {!d && t.last_words && <div class="hc-head">{t.last_words}</div>}
      <div class="hc-meta">
        {t.place} · {t.node} · {t.state}{t.cost != null ? ` · ${money(t.cost)}` : ""}
        <br />started {ago(t.started)} · last step {ago(t.last_step)}
      </div>
    </div>
  );
}

/** The drag handle on the sidebar's right edge. */
function Grip() {
  const down = (e: PointerEvent) => {
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    const x0 = e.clientX, w0 = sideWidth.value;
    el.setPointerCapture(e.pointerId);
    document.body.classList.add("resizing");
    hideNow();
    const move = (m: PointerEvent) => setSideWidth(w0 + m.clientX - x0, false);
    const up = () => {
      el.removeEventListener("pointermove", move); el.removeEventListener("pointerup", up); el.removeEventListener("pointercancel", up);
      document.body.classList.remove("resizing");
      setSideWidth(sideWidth.value, true);
    };
    el.addEventListener("pointermove", move); el.addEventListener("pointerup", up); el.addEventListener("pointercancel", up);
  };
  const key = (e: KeyboardEvent) => {
    if (e.key === "ArrowLeft") setSideWidth(sideWidth.value - 20, true);
    else if (e.key === "ArrowRight") setSideWidth(sideWidth.value + 20, true);
  };
  return <div class="grip" role="separator" aria-orientation="vertical" aria-label="Resize the sidebar (double-click resets)" tabIndex={0} title="Drag to resize; double-click resets" onPointerDown={down} onDblClick={resetSideWidth} onKeyDown={key} />;
}

/** A room's row (design §2.27): ⇄ and how many talk in it, and a council's tag. */
function RoomBadge({ t }: { t: ThreadItem }) {
  const ps = t.participants ?? [];
  const who = ps.map((p) => `${p.name} (@${p.slug}${p.muted ? ", muted" : ""})`).join(", ");
  return (
    <>
      <span class="roomtag" title={`A room${who ? ": " + who : ""}`}>⇄ {ps.length}</span>
      {t.room_mode === "parallel" && <span class="roomtag council" title="Council: everyone answers at once">council</span>}
    </>
  );
}

/** Folds the user changed; the rest follow the default (hot trees open). */
const folds = signal<Record<string, boolean>>({});

function Row({ n, depth, tree, place }: { n: H.TreeNode; depth: number; tree: boolean; place: boolean }) {
  const t = n.t;
  const kids = tree ? n.kids : [];
  const isOpen = folds.value[t.id] ?? (kids.length > 0 && H.hot(n));
  const total = tree && kids.length ? H.treeCost(n) : t.cost ?? 0;
  const hiddenHot = kids.length > 0 && !isOpen && kids.some(H.hot);
  return (
    <>
      <div class={"row" + (S.selected.value === t.id && !S.homeView.value ? " sel" : "") + (t.unread ? " unread" : "")} style={`padding-left:${6 + depth * 14}px`}>
        {kids.length > 0 ? (
          <button class="fold" title={isOpen ? "Collapse" : "Expand"} onClick={() => (folds.value = { ...folds.value, [t.id]: !isOpen })}>{isOpen ? "▾" : "▸"}</button>
        ) : <span class="fold" />}
        <button class="rbtn" data-tid={t.id} onPointerEnter={(e) => enter(t, e)} onPointerLeave={leave} onFocus={(e) => focused(t, e)} onBlur={hideNow} onClick={() => void S.select(t.id)}>
          <span class={"dot " + dotOf(t)} />
          <span class="rt">
            <span class="title">{t.pinned ? "📌 " : ""}{t.detached ? titleOf(t) : t.title}{t.room && <RoomBadge t={t} />}{t.unread && <span class="unr" title="Unread" />}{(t.digest?.asks ?? 0) > 0 && <span class="asks" title={asksTitle(t.digest!.asks)}>asks</span>}</span>
            <span class="meta">
              {n.under ? `in ${n.under} · ` : ""}{place || t.place !== n.t.place ? `${t.place} · ` : ""}{t.node} · {short(t.last_step)}
              {total ? ` · ${money(total)}` : ""}{kids.length ? ` · ${kids.length} spawned` : ""}{hiddenHot ? " · ●" : ""}
            </span>
            {t.digest
              ? <span class={"words dg" + (t.digest.fresh ? "" : " stale")} title={t.digest.fresh ? undefined : "stale: the conversation moved since this digest"}>{t.digest.headline}</span>
              : <span class="words">{t.last_words ?? ""}</span>}
          </span>
        </button>
      </div>
      {isOpen && kids.map((k) => <Row key={k.t.id} n={k} depth={depth + 1} tree place={place} />)}
    </>
  );
}

function Head({ s, open, onToggle }: { s: Section; open: boolean; onToggle: () => void }) {
  return (
    <div class="sec">
      <button class="sectoggle" aria-expanded={open} title={open ? "Collapse" : "Expand"} onClick={onToggle}>
        <span class="fold">{open ? "▾" : "▸"}</span>
        <span class="sname">{s.label}</span>
        {s.memoryOff && <span class="memoff" title="memory off">∅</span>}
        {s.needs && <span class="dot waiting" title="Something here waits on you" />}
        {s.working && <span class="dot working" title="Something here is working" />}
      </button>
      {s.unread > 0 && <button class="markread" title={`Mark everything in ${s.label} read`} onClick={() => void (s.time ? S.markUnreadIn(s.unreadRows) : S.markAllRead(s.place))}>✓</button>}
      {s.unread > 0 && <span class="sunr" title={`${s.unread} unread`}>{s.unread} new</span>}
      <span class="scount">{s.count}</span>
    </div>
  );
}

function AppUnpin({ id, name }: { id: string; name: string }) {
  const [busy, setBusy] = useState(false);
  return <button class="appunpin" title={`Unpin ${name}`} aria-label={`Unpin ${name}`} disabled={busy} onClick={async () => {
    setBusy(true);
    try { await S.unpinApp(id); } finally { setBusy(false); }
  }}>Unpin</button>;
}

/** The pinned apps, a section that folds (closed by default); each opens
 * in a tab of its own. */
function Apps() {
  const list = S.apps.value;
  if (!list.length) return null;
  const open = S.appsOpen.value;
  return (
    <div class="section apps">
      <div class="sec">
        <button class="sectoggle" aria-expanded={open} title={open ? "Collapse" : "Expand"} onClick={() => S.setAppsOpen(!open)}>
          <span class="fold">{open ? "▾" : "▸"}</span>
          <span class="sname">Apps</span>
        </button>
        <span class="scount">{list.length}</span>
      </div>
      {open && (
        <div class="applist">
          {list.map((a) => (
            <div key={a.id} class="approw">
              <button class="appchip" title={`Open ${a.name} in a new tab`} onClick={() => void S.openApp(a)}>
                <span class="appicon">{(a.name.trim()[0] ?? "?").toUpperCase()}</span>
                <span class="appname">{a.name}</span>
              </button>
              <AppUnpin id={a.id} name={a.name} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function Sidebar({ onNew, onNewRoom }: { onNew: () => void; onNewRoom?: () => void }) {
  useEffect(() => useTick(), []);
  useEffect(() => {
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") hideNow(); };
    document.addEventListener("keydown", esc);
    return () => document.removeEventListener("keydown", esc);
  }, []);
  void now.value;
  const q = S.filter.value.toLowerCase();
  const needs = H.needsCount.value;
  const more = Object.values(S.placeRows.value).flat();
  const order = S.sideOrder.value;
  const g = group(S.threads.value, more, S.placeCounts.value, q, dotOf, order);
  const choice = S.placeOpen.value;
  // Spawns under a pinned thread count too (only its root did).
  const anyUnread = g.pinned.some((n) => { let u = false; H.walk(n, (t) => { if (t.unread) u = true; }); return u; }) || g.sections.some((s) => s.unread > 0);
  // A filter shows every match; else the user's choice, or the default.
  const isOpen = (s: Section, i: number) => q !== "" || (choice[s.key] ?? openByDefault(s, i === 0));
  const pinOpen = q !== "" || S.pinnedOpen.value;
  const anyOpen = !q && (S.pinnedOpen.value && g.pinned.length > 0 || S.appsOpen.value || g.sections.some(isOpen));
  // An open section the window does not fully cover reads its place (once; a refresh follows it).
  useEffect(() => {
    if (q) return;
    g.sections.forEach((s, i) => {
      if (!s.time && isOpen(s, i) && s.loaded < s.count && !(s.place in S.placeRows.value)) void S.readPlace(s.place);
    });
  });
  return (
    <aside style={`--side-w:${sideWidth.value}px`}>
      <Grip />
      <Card />
      <div class="side-top">
        <button class={"apptitle" + (S.homeView.value ? " sel" : "")} onClick={() => S.goHome()}>
          Stitch
          {needs > 0 && <span class="badge" title="Waiting on you">{needs}</span>}
        </button>
        {anyUnread && <button class="markall" title="Mark every conversation read" onClick={() => { if (confirm("Mark every conversation as read?")) void S.markAllRead(); }}>✓ all read</button>}
        {mapShown.value && <button class="collapse" title="Hide the sidebar (the map takes the width)" aria-label="Hide the sidebar" onClick={toggleSide}>«</button>}
        {onNewRoom && <button class="newroom" title="New room: models and threads talking together (design §2.27)" onClick={onNewRoom}>＋ Room</button>}
        <button class="primary" title="New thread" onClick={onNew}>＋</button>
      </div>
      <div class="side-head">
        <input placeholder="Filter threads" value={S.filter.value} onInput={(e) => (S.filter.value = e.currentTarget.value)} />
        <button class="collapseall" title="Collapse every section" aria-label="Collapse all" disabled={!anyOpen} onClick={() => S.collapseAll(g.sections.map((s) => s.key))}>⊟</button>
        <button class="order" title={order === "place" ? "Grouped by project: group by time instead" : "Grouped by time: group by project instead"} aria-label={order === "place" ? "Group by time" : "Group by project"} onClick={() => S.setSideOrder(order === "place" ? "time" : "place")}>{order === "place" ? "▤" : "◷"}</button>
      </div>
      <SearchBox />
      <div class="rows" onScroll={hideNow}>
        {!q && <Apps />}
        {g.pinned.length > 0 && (
          <div class="section">
            <div class="sec">
              <button class="sectoggle" aria-expanded={pinOpen} title={pinOpen ? "Collapse" : "Expand"} onClick={() => S.setPinnedOpen(!pinOpen)}>
                <span class="fold">{pinOpen ? "▾" : "▸"}</span>
                <span class="sname">Pinned</span>
                {!pinOpen && g.pinned.some((n) => H.hot(n)) && <span class="dot working" title="Something here is working or waits on you" />}
              </button>
              <span class="scount">{g.pinned.length}</span>
            </div>
            {pinOpen && g.pinned.map((n) => <Row key={n.t.id} n={n} depth={0} tree={!q} place />)}
          </div>
        )}
        {g.sections.map((s, i) => {
          const open = isOpen(s, i);
          return (
            <div class="section" key={s.key}>
              <Head s={s} open={open} onToggle={() => S.setPlaceOpen(s.key, !open)} />
              {open && s.nodes.map((n) => <Row key={n.t.id} n={n} depth={0} tree={!q} place={s.time} />)}
              {open && !q && !s.time && s.loaded < s.count && !(s.place in S.placeRows.value) && <div class="meta" style="padding:4px 12px">Loading…</div>}
            </div>
          );
        })}
        {q && !g.pinned.length && !g.sections.length && <div class="meta" style="padding:12px">No thread matches.</div>}
      </div>
    </aside>
  );
}
