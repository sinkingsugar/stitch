// The map's detail card: what a place, an effort or a thread is, with an Open
// button, and the spawned tree whose rows select (and open).
import { useState } from "preact/hooks";
import * as S from "../store";
import * as A from "../actions/state";
import * as M from "./state";
import { attachTarget } from "./ScopeTray";
import { openNewRoom } from "../room/state";
import { scopeOfThread, toggleScope, type StagedScope } from "../scopes";
import { ago, depthOf, flatten, fmtTime, money, placeName, resolve, selOfThread, stateLabel, subCount, threadOf, kindOf, type Effort, type MThread, type Place, type Sel } from "./model";

const TREE_FOLD = 30;
/** Trees shown whole, per effort, while the page lives. */
const expanded = new Set<string>();

const Dot = ({ t }: { t: { need: boolean; working: boolean } }) => <i class={"mdot " + (kindOf(t) === "need" ? "need" : kindOf(t) === "work" ? "work" : "idle")} style="margin-right:6px" />;
const Sep = () => <span class="sep">·</span>;

function Tree({ root, cur, eff, bump }: { root: MThread; cur: string | null; eff: Effort; bump: () => void }) {
  const all = flatten(root);
  const open = expanded.has(root.id);
  const rows = open ? all : all.slice(0, TREE_FOLD);
  return (
    <div class="tree">
      {rows.map(([t, d]) => {
        const n = t.kids.length ? subCount(t) : 0;
        const away = !t.synthetic && t.placeKey !== eff.place;
        return (
          <div key={t.id} class={"row" + (t.id === cur ? " cur" : "")} style={`padding-left:${6 + Math.min(d, 12) * 14}px`}
            onClick={() => M.select(selOfThread(t), "card")} onDblClick={() => M.openSel(selOfThread(t))}>
            <Dot t={t} /><span class="t">{t.title || t.id}</span>
            {away && <span class="tag" title={t.placeKey}>{placeName(t.placeKey)}</span>}
            {n > 0 && <span class="n">{n}</span>}
            {!t.synthetic && <button class="ropen" title="Open this thread" onClick={(e) => { e.stopPropagation(); M.openSel(selOfThread(t)); }}>Open</button>}
          </div>
        );
      })}
      {all.length > TREE_FOLD && (
        <div class="row more" onClick={() => { if (open) expanded.delete(root.id); else expanded.add(root.id); bump(); }}>{open ? "show fewer" : `show all ${all.length}`}</div>
      )}
    </div>
  );
}

function PlaceCard({ p }: { p: Place }) {
  const needs = p.efforts.filter((e) => e.need).slice(0, 6);
  const recent = p.efforts.slice().sort((a, b) => b.e - a.e).slice(0, 8);
  const li = (e: Effort) => <div key={e.id} class="row" onClick={() => M.select({ kind: "effort", id: e.id }, "card")}><Dot t={e} /><span class="t">{e.root.title || e.id}</span></div>;
  return (
    <>
      <h3>{p.name}</h3>
      <div class="meta">{p.key}</div>
      <div class="meta">
        {p.efforts.length} efforts<Sep />{p.count} threads<Sep />{money(p.cost)}
        {p.working ? <><Sep />{p.working} working</> : null}
        {p.need ? <><Sep /><b class="needs">{p.need} need you</b></> : null}
      </div>
      <div class="meta">{fmtTime(p.s)} → {ago(p.e)}</div>
      {needs.length > 0 && <><div class="lbl">Needs you</div><div class="tree">{needs.map(li)}</div></>}
      <div class="lbl">Recent</div><div class="tree">{recent.map(li)}</div>
    </>
  );
}

function ThreadCard({ t, s }: { t: MThread; s: Sel }) {
  const [, setN] = useState(0);
  const e = t.effort;
  const q = t.question;
  if (t.synthetic) {
    return (
      <>
        <h3>{t.title}</h3>
        <div class="meta">{placeName(t.placeKey)}<Sep />{t.kids.length} threads name a parent this listing does not have</div>
        <div class="goal">Their parent was deleted, lives on a repository this node does not hold, or the node is older than <code>conversations</code>.</div>
        {e && e.threads.length > 1 && <><div class="lbl">Threads · {e.threads.length - 1}</div><Tree root={t} cur={null} eff={e} bump={() => setN((x) => x + 1)} /></>}
      </>
    );
  }
  const head = t.headline || t.lastWords;
  const goal = String(t.goal || "").trim().split("\n").slice(0, 4).join("\n");
  const row = S.threads.value.find((r) => r.id === t.id);
  void s;
  return (
    <>
      <h3>{t.title || t.id}</h3>
      <div class="meta">
        {placeName(t.placeKey)}{t.node ? <><Sep />{t.node}</> : null}<Sep /><Dot t={t} />{stateLabel(t)}<Sep />{money(t.cost)}
        {t.unread ? <><Sep /><span class="unr">unread</span></> : null}
      </div>
      <div class="meta">{fmtTime(t.s)} → {fmtTime(t.e)} <span class="dim">({ago(t.e)})</span></div>
      <button class="primary open" onClick={() => M.openSel(s)}>Open</button>
      <AskRow t={t} />
      {q && (
        <div class="approval ask">
          <div class="q">Allow this command?</div>
          <code>{q.command}</code>
          <div class="btns">
            <button class="primary" onClick={() => void S.answer(q.id, true)}>Allow</button>
            <button onClick={() => void S.answer(q.id, true, true)}>Always</button>
            <button class="danger" onClick={() => void S.answer(q.id, false)}>Decline</button>
          </div>
        </div>
      )}
      {!q && t.need && <div class="head asks">Waiting on you{row?.outcome ? `: ${row.outcome}` : ""}. Answer it in the conversation, or in Needs you.</div>}
      {head && <div class="head">{head}</div>}
      {goal && goal !== head && <div class="goal">{goal}</div>}
      {e && e.threads.length > 1 && (
        <>
          <div class="lbl">Spawned threads · {e.threads.length - 1}<span class="dim"> · depth {depthOf(e.root)}</span></div>
          <Tree root={e.root} cur={t.id} eff={e} bump={() => setN((x) => x + 1)} />
        </>
      )}
      <div class="id"><code>{t.id}</code><CopyId id={t.id} /></div>
    </>
  );
}

/** "Ask about" this effort or thread (a new chat with it attached), attach it
 * to the chat last open, or put it in the map's scope tray (design §2.27). */
function AskRow({ t }: { t: MThread }) {
  const sc = scopeOfThread(t);
  if (!sc) return null;
  const on = M.picked.value.some((x) => x.kind === sc.kind && x.id === sc.id);
  const to = attachTarget();
  const what = sc.kind === "effort" ? "effort" : "thread";
  const ask = (l: StagedScope[]) => void A.askInNew(l, t.placeKey);
  return (
    <div class="askrow">
      <button title={sc.kind === "effort" ? `A new chat with this effort attached: its ${sc.threads} threads' briefing` : "A new chat with this conversation attached"} onClick={() => ask([sc])}>Ask about {what}</button>
      {to && to.id !== t.id && <button title={`Attach this ${what} to “${to.title}” and go back to it`} onClick={() => void A.attachTo(to.id, [sc])}>Attach to chat</button>}
      <button class="room" title={`A room of models (and, if you like, this thread itself) briefed on this ${what}`} onClick={() => openNewRoom({ scopes: [sc], place: t.placeKey, offer: [{ id: t.id, title: t.title || t.id }], pullable: sc.kind === "thread" && !t.kids.length ? { id: t.id, title: t.title || t.id } : null })}>Open room…</button>
      <button class={"pick" + (on ? " on" : "")} aria-pressed={on} title={on ? "Take it out of the picked set" : "Pick it (or Shift+click on the map) to ask about several at once"} onClick={() => (M.picked.value = toggleScope(M.picked.value, sc))}>{on ? "✓ Picked" : "+ Scope"}</button>
    </div>
  );
}

function CopyId({ id }: { id: string }) {
  const [done, setDone] = useState(false);
  return <button onClick={() => { try { void navigator.clipboard.writeText(id); setDone(true); setTimeout(() => setDone(false), 1200); } catch { /* */ } }}>{done ? "copied" : "copy"}</button>;
}

export function Card() {
  const m = M.model.value;
  void M.stats.value;
  // The hover previews only while nothing is selected: a selected card stays put, so its buttons can be reached.
  const s = M.sel.value ?? M.hover.value;
  const o = resolve(m, s);
  if (!s || !o) return null;
  const pinned = s === M.sel.value;
  return (
    <aside class={"mapcard" + (pinned ? "" : " preview")} aria-label="Details">
      {pinned && <button class="x" aria-label="Close" onClick={() => M.select(null, "ui")}>×</button>}
      {s.kind === "place" ? <PlaceCard p={o as Place} /> : <ThreadCard key={s.id} t={threadOf(m, s)!} s={s} />}
    </aside>
  );
}
