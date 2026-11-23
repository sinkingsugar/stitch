// Scopes in the composer and on messages (design §2.27): the staged chips
// above the text, the ⎇ picker (search the threads and efforts, or paste an
// ID), and the chips of a sent message, which open the thread or show the
// effort on the map.
import "./scopes.css";
import { useEffect, useRef, useState } from "preact/hooks";
import * as S from "../store";
import * as M from "../map/state";
import { setHomeMode } from "../nav";
import { candidates, parseScope, scopeKey, scopeLabel, type StagedScope } from "../scopes";
import type { ScopeChip } from "../payload";

/** The staged scopes, removable. */
export function StagedScopes({ scopes, onRemove }: { scopes: StagedScope[]; onRemove: (s: StagedScope) => void }) {
  if (!scopes.length) return null;
  return (
    <div class="scopechips staged-scopes" aria-label="Attached threads and efforts">
      {scopes.map((s) => (
        <span key={scopeKey(s)} class={"scopechip " + s.kind} title={`${s.kind}:${s.id}`}>
          <span class="sym" aria-hidden="true">⎇</span> <span class="lab">{scopeLabel(s)}</span>
          <button class="rm" title="Remove" aria-label={`Remove ${scopeLabel(s)}`} onClick={() => onRemove(s)}>✕</button>
        </span>
      ))}
    </div>
  );
}

/** The ⎇ button and its popover: attach a thread, or its whole effort. */
export function ScopePicker({ scopes, onToggle }: { scopes: StagedScope[]; onToggle: (s: StagedScope) => void }) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const box = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!open) return;
    setTimeout(() => box.current?.focus(), 0);
    // The whole forest when the map has it; reading it is one request.
    if (!M.loaded.peek()) void M.load();
  }, [open]);
  const on = (s: StagedScope) => scopes.some((x) => x.kind === s.kind && x.id === s.id);
  const pasted = parseScope(q);
  const list = open ? candidates(q, S.threads.value, M.loaded.value ? M.model.value : null) : [];
  const close = () => { setOpen(false); setQ(""); };
  return (
    <div class="menu scopepick">
      <button title="Attach a thread or an effort (its briefing goes with the message)" aria-label="Attach a thread or effort" aria-expanded={open} onClick={() => setOpen(!open)}>⎇</button>
      {open && (
        <>
          <div class="scrim" onClick={close} />
          <div class="mlist up scopelist" role="dialog" aria-label="Attach a thread or effort" onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); close(); } }}>
            <input ref={box} type="search" placeholder="Search threads, or paste an ID / effort:<id>" value={q}
              onInput={(e) => setQ(e.currentTarget.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && pasted) { onToggle(pasted); setQ(""); } }} />
            {pasted && (
              <button class={"mi" + (on(pasted) ? " on" : "")} onClick={() => { onToggle(pasted); setQ(""); }}>
                {on(pasted) ? "✓ " : "+ "}{pasted.kind === "effort" ? "Effort" : "Thread"} <code>{pasted.id}</code>
              </button>
            )}
            {list.map((c) => (
              <div key={c.id} class="scand">
                <div class="ct"><span class="t">{c.title}</span><span class="meta">{c.place}</span></div>
                <button class={"sbtn" + (on(c.thread) ? " on" : "")} aria-pressed={on(c.thread)} title="Attach this conversation alone" onClick={() => onToggle(c.thread)}>{on(c.thread) ? "✓ " : ""}Thread</button>
                {c.effort && (
                  <button class={"sbtn" + (on(c.effort) ? " on" : "")} aria-pressed={on(c.effort)} title={`Attach the whole effort: ${c.effort.title} and every thread spawned under it`} onClick={() => onToggle(c.effort!)}>
                    {on(c.effort) ? "✓ " : ""}Effort{c.effort.threads ? ` (${c.effort.threads})` : ""}
                  </button>
                )}
              </div>
            ))}
            {!list.length && !pasted && <div class="meta" style="padding:6px">{M.loading.value ? "Loading threads…" : "No thread matches."}</div>}
          </div>
        </>
      )}
    </div>
  );
}

/** Shows an effort on the map, selected (the conversation closed). */
export function showEffort(id: string) {
  const m = M.model.peek();
  const t = m.byId.get(id);
  const eff = t ? t.rootId : id;
  S.goHome();
  setHomeMode("map");
  const go = () => {
    const mm = M.model.peek();
    const e = mm.eById.get(eff);
    if (e && !e.vis) M.setFilter({ time: "all" });
    M.select({ kind: "effort", id: eff }, "ui");
  };
  if (M.loaded.peek()) go();
  else void M.load().then(go);
}

/** The chips of a sent message: a thread's opens it, an effort's shows it on the map (↗ opens its conversation). */
export function ScopeChips({ scopes }: { scopes: ScopeChip[] | undefined }) {
  if (!scopes?.length) return null;
  return (
    <div class="scopechips sent" aria-label="Attached threads and efforts">
      {scopes.map((s) => {
        const label = scopeLabel(s);
        return s.kind === "effort" ? (
          <span key={scopeKey(s)} class="scopechip effort">
            <button class="go" title={`Show this effort on the map (effort:${s.id})`} onClick={() => showEffort(s.id)}><span class="sym" aria-hidden="true">⎇</span> {label}</button>
            <button class="rm" title="Open its conversation" aria-label={`Open ${s.title ?? s.id}`} onClick={() => void S.select(s.id)}>↗</button>
          </span>
        ) : (
          <span key={scopeKey(s)} class="scopechip thread">
            <button class="go" title={`Open this conversation (thread:${s.id})`} onClick={() => void S.select(s.id)}><span class="sym" aria-hidden="true">⎇</span> {label}</button>
          </span>
        );
      })}
    </div>
  );
}
