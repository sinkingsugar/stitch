// The thread header's menu and the dialogs its items open.
import "./actions.css";
import { useEffect, useState } from "preact/hooks";
import * as S from "../store";
import * as A from "./state";
import { explain, label, type Doomed } from "./doomed";
import * as M from "../map/state";
import { askScope } from "../scopes";
import { openNewRoom, pullIn } from "../room/state";
import type { ThreadItem, Entry } from "../model";
import type { SetItem } from "../api/SetItem";

/** The "forked from" and "parent" links, under the title. */
export function HeaderLinks({ t }: { t: ThreadItem }) {
  const forked = S.forkedFrom.value && !t.parent && S.threads.value.some((x) => x.id === S.forkedFrom.value!.thread);
  if (!forked && !t.parent) return null;
  return (
    <div class="hlinks">
      {S.forkedFrom.value && !t.parent && S.threads.value.some((x) => x.id === S.forkedFrom.value!.thread) && (
        <button class="link" title={S.forkedFrom.value.title ?? S.forkedFrom.value.thread} onClick={() => void S.select(S.forkedFrom.value!.thread)}>⑂ forked from {(S.forkedFrom.value.title ?? "source").slice(0, 28)}</button>
      )}
      {t.parent && (
        <button class="link" title={t.parent_title ?? t.parent} onClick={() => void S.select(t.parent!)}>↰ parent</button>
      )}
    </div>
  );
}

/** The controls of the header (pin, mode, the menu button): one line. */
export function HeaderExtras({ t }: { t: ThreadItem }) {
  const open = A.menuOpen.value;
  const item = (label: string, f: () => void, cls = "") => (
    <button class={"mi " + cls} onClick={() => { A.menuOpen.value = false; f(); }}>{label}</button>
  );
  return (
    <>
      {t.pinned && <span title="Pinned">📌</span>}
      {t.read_only && <span class="chip" title="Read-only: it may not change files">read-only</span>}
      {!t.room && <button class="pullin" title="Pull in model…: a second opinion, in a room with this chat" aria-label="Pull in model" onClick={() => pullIn(t)}>⇄＋</button>}
      <div class="menu">
        <button title="Thread actions" onClick={() => (A.menuOpen.value = !open)}>⋯</button>
        {open && (
          <>
            <div class="scrim" onClick={() => (A.menuOpen.value = false)} />
            <div class="mlist">
              {item("Rename…", () => (A.dialog.value = { k: "rename" }))}
              {item(t.pinned ? "Unpin" : "Pin", () => void A.pin(t.id, !t.pinned))}
              {item("Fork…", () => (A.dialog.value = { k: "fork", upto: null, preset: "fork" }))}
              {item("Compact… (summary + last 10)", () => (A.dialog.value = { k: "fork", upto: null, preset: "compact" }))}
              {item("Summarize & continue", () => (A.dialog.value = { k: "fork", upto: null, preset: "summarize" }))}
              <hr />
              {(() => {
                const sc = askScope(t.id, t.title, M.loaded.value ? M.model.value : null, S.threads.value);
                return item(sc.kind === "effort" ? `Ask about this effort${sc.threads ? ` (${sc.threads} threads)` : ""}…` : "Ask about this thread…", () => void A.askInNew([sc], t.place));
              })()}
              {!t.room && item("Pull in model… (a room with this chat)", () => pullIn(t))}
              {!t.room && (() => {
                const sc = askScope(t.id, t.title, M.loaded.value ? M.model.value : null, S.threads.value);
                return item("Open a room about this…", () => openNewRoom({ scopes: [sc], place: t.place, offer: [{ id: t.id, title: t.title }] }));
              })()}
              <hr />
              {item(t.read_only ? "Make writable" : "Make read-only", () => void A.readOnly(t.id, !t.read_only))}
              {item("Tools & skills…", () => (A.dialog.value = { k: "tools" }))}
              <hr />
              {item("Copy transcript as Markdown", () => void A.copy(A.markdown(t.title, S.entries.value)))}
              {item("Delete…", () => (A.dialog.value = { k: "delete" }), "danger")}
            </div>
          </>
        )}
      </div>
    </>
  );
}

function Modal({ title, children, onClose }: { title: string; children: preact.ComponentChildren; onClose: () => void }) {
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    addEventListener("keydown", k);
    return () => removeEventListener("keydown", k);
  }, []);
  return (
    <div class="modal" onClick={onClose}>
      <div class="card" onClick={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        {children}
      </div>
    </div>
  );
}
const close = () => (A.dialog.value = null);

function Rename({ t }: { t: ThreadItem }) {
  const [v, setV] = useState(t.title);
  const go = async () => { close(); if (v.trim()) await A.rename(t.id, v.trim()); };
  return (
    <Modal title="Rename thread" onClose={close}>
      <input autofocus value={v} onInput={(e) => setV(e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && void go()} />
      <div class="btns end"><button onClick={close}>Cancel</button><button class="primary" onClick={() => void go()}>Rename</button></div>
    </Modal>
  );
}

/** What a fork keeps, estimated from the transcript here (as the TUI's form does). */
function keeps(items: Entry[], upto: string | null, last: number | null, summarize: boolean) {
  let msgs: string[] = [];
  for (const e of items) if ((e.kind === "user" || e.kind === "assistant") && e.id) msgs.push(e.id);
  if (upto && msgs.includes(upto)) msgs = msgs.slice(0, msgs.indexOf(upto) + 1);
  const all = msgs.length;
  let kept = all;
  if (last != null) kept = Math.min(last, all);
  else if (summarize) kept = 0;
  return { all, kept, summarized: summarize ? all - kept : 0 };
}

function Fork({ t, d }: { t: ThreadItem; d: Extract<A.Dialog, { k: "fork" }> }) {
  const [lastOn, setLastOn] = useState(d.preset === "compact");
  const [n, setN] = useState(10);
  const [noAtt, setNoAtt] = useState(false);
  const [noTools, setNoTools] = useState(false);
  const [sum, setSum] = useState(d.preset !== "fork");
  const last = lastOn ? Math.max(1, n | 0) : null;
  const k = keeps(S.entries.value, d.upto, last, sum);
  const refused = (k.kept === 0 && !sum) || (sum && k.summarized === 0);
  const go = async () => {
    close();
    await A.fork(t.id, { upto: d.upto, last, no_attachments: noAtt, no_tools: noTools, summarize: sum });
  };
  const title = d.preset === "compact" ? "Compact" : d.preset === "summarize" ? "Summarize & continue" : "Fork";
  return (
    <Modal title={`${title}${d.upto ? ` from ${d.upto}` : ""}`} onClose={close}>
      <div class="frm">
        <label><input type="radio" checked={!lastOn} onChange={() => setLastOn(false)} /> All messages{d.upto ? ` up to ${d.upto}` : ""}</label>
        <label>
          <input type="radio" checked={lastOn} onChange={() => setLastOn(true)} /> Only the last{" "}
          <input class="num" type="number" min={1} value={n} onInput={(e) => { setN(+e.currentTarget.value); setLastOn(true); }} /> messages
        </label>
        <label><input type="checkbox" checked={noAtt} onChange={(e) => setNoAtt(e.currentTarget.checked)} /> Without attachments</label>
        <label><input type="checkbox" checked={noTools} onChange={(e) => setNoTools(e.currentTarget.checked)} /> Without tool calls</label>
        <label><input type="checkbox" checked={sum} onChange={(e) => setSum(e.currentTarget.checked)} /> Summarize what is not kept (a model call)</label>
      </div>
      <div class="meta">
        Keeps {k.kept} of {k.all} messages{sum ? ` + a summary of ${k.summarized}` : ""}.
        {refused && <span class="warn"> The node would refuse this: {sum ? "nothing is left for a summary" : "nothing is kept"}.</span>}
      </div>
      <div class="meta">A fork reads what it keeps from this thread: its own transcript shows what is said after the fork (and the summary).</div>
      <div class="btns end"><button onClick={close}>Cancel</button><button class="primary" disabled={refused} onClick={() => void go()}>{title}</button></div>
    </Modal>
  );
}

function Delete({ t }: { t: ThreadItem }) {
  const [list, setList] = useState<Doomed[] | null>(null);
  const [err, setErr] = useState("");
  useEffect(() => { A.doomed(t.id).then(setList, (e) => setErr(String(e))); }, []);
  const e = list?.length ? explain(list) : null;
  return (
    <Modal title={e?.title ?? `Delete “${t.title}” for good?`} onClose={close}>
      {e && <div class="meta">{e.detail}</div>}
      {e?.showList && <ul class="doomed">{list!.map((x) => <li key={x.id}><b>{x.title}</b> {label(x) && <span class={x.role === "this" ? "role this" : "role"}>{label(x)}</span>} <span class="meta">{x.id}</span></li>)}</ul>}
      {!list && !err && <div class="meta">Checking what goes…</div>}
      {err && <div class="warn">{err}</div>}
      <div class="btns end"><button onClick={close}>Cancel</button>
        <button class="danger" disabled={!list?.length} onClick={() => { close(); void A.remove(t.id); }}>{list && list.length > 1 ? `Delete all ${list.length}` : "Delete"}</button></div>
    </Modal>
  );
}

function Rewind({ t, d }: { t: ThreadItem; d: Extract<A.Dialog, { k: "rewind" }> }) {
  return (
    <Modal title={`Rewind to ${d.id}?`} onClose={close}>
      <div class="meta">The thread goes on from this message; what came after is set aside (kept, off the line).</div>
      <blockquote class="q">{d.text.slice(0, 280)}</blockquote>
      <div class="btns end"><button onClick={close}>Cancel</button>
        <button class="danger" onClick={() => { close(); void A.rewind(t.id, d.id); }}>Rewind</button></div>
    </Modal>
  );
}

function Tools({ t }: { t: ThreadItem }) {
  const [sets, setSets] = useState<A.Sets | null>(null);
  const [err, setErr] = useState("");
  useEffect(() => { A.toolset(t.id).then(setSets, (e) => setErr(String(e))); }, []);
  const toggle = async (it: SetItem, skill: boolean) => {
    const c = [(it.on ? "-" : "+") + it.name];
    try { setSets(await A.toolset(t.id, skill ? [] : c, skill ? c : [])); } catch (e) { setErr(String(e)); }
  };
  const list = (items: SetItem[], skill: boolean) =>
    items.map((it) => (
      <label key={it.name} class={"mrow" + (it.available ? "" : " off")} title={it.note}>
        <input type="checkbox" checked={it.on} onChange={() => void toggle(it, skill)} /> <span>{it.name}</span>
        <span class="meta">{it.kind}{it.node ? ` · ${it.node}` : ""}{it.available ? "" : " · unavailable here"} {it.note.slice(0, 70)}</span>
      </label>
    ));
  return (
    <Modal title="Tools & skills" onClose={close}>
      {!sets && !err && <div class="meta">Loading…</div>}
      {err && <div class="warn">{err}</div>}
      {sets && <div class="list"><h4>Tools</h4>{list(sets.tools, false)}<h4>Skills</h4>{list(sets.skills, true)}</div>}
      <div class="btns end"><button class="primary" onClick={close}>Done</button></div>
    </Modal>
  );
}

export function ActionDialogs() {
  const d = A.dialog.value;
  const t = S.current.value;
  const b = A.busy.value;
  return (
    <>
      {d && t && d.k === "rename" && <Rename t={t} />}
      {d && t && d.k === "fork" && <Fork t={t} d={d} />}
      {d && t && d.k === "delete" && <Delete t={t} />}
      {d && t && d.k === "rewind" && <Rewind t={t} d={d} />}
      {d && t && d.k === "tools" && <Tools t={t} />}
      {b && <div class="toast">{b}…</div>}
    </>
  );
}
