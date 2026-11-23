// The map's scope tray (design §2.27, rooms.md §5): what was picked with
// Shift+click or "+ Scope", to ask about in a new chat, attach to the
// conversation last open, or clear.
import "../transcript/scopes.css";
import * as S from "../store";
import * as A from "../actions/state";
import * as M from "./state";
import { selOfThread } from "./model";
import { openNewRoom } from "../room/state";
import { scopeKey, scopeLabel, scopeThreads, type StagedScope } from "../scopes";

/** The place a new chat about these goes in: theirs when they share one, else none (the form's choice). */
function placeOf(l: StagedScope[]): string | null {
  const m = M.model.peek();
  const ps = new Set(l.map((s) => m.byId.get(s.id)?.placeKey).filter(Boolean));
  return ps.size === 1 ? [...ps][0]! : null;
}

/** The conversation "Attach to chat" goes to: the one last open, when the list has it. */
export function attachTarget(): { id: string; title: string } | null {
  const id = S.lastOpen.value;
  if (!id) return null;
  const row = S.threads.value.find((t) => t.id === id || t.earlier?.includes(id)) ?? S.extra.value[id] ?? null;
  return { id: row?.id ?? id, title: row?.title || id };
}

export function ScopeTray() {
  const l = M.picked.value;
  if (!l.length) return null;
  const n = scopeThreads(l);
  const to = attachTarget();
  const done = () => (M.picked.value = []);
  return (
    <div class="scopetray" role="region" aria-label="Picked for a chat">
      <span class="count"><b>{l.length}</b> picked <span class="dim">({n} thread{n === 1 ? "" : "s"})</span></span>
      <span class="chips">
        {l.map((s) => (
          <span key={scopeKey(s)} class={"scopechip mini " + s.kind} title={`${s.kind}:${s.id}`}>
            <button class="go" title="Show it" onClick={() => { const t = M.model.peek().byId.get(s.id); if (t) M.select(selOfThread(t), "ui"); }}>⎇ {scopeLabel(s)}</button>
            <button class="rm" title="Remove" aria-label={`Remove ${scopeLabel(s)}`} onClick={() => (M.picked.value = l.filter((x) => x !== s))}>✕</button>
          </span>
        ))}
      </span>
      <span class="acts">
        <button class="primary" title="A new thread with these attached" onClick={() => { const p = placeOf(l); done(); void A.askInNew(l, p); }}>Ask in new chat</button>
        <button title="A room of models briefed on these (and, if you like, with the picked threads themselves in it)" onClick={() => { const p = placeOf(l); done(); openNewRoom({ scopes: l, place: p, offer: l.map((s) => ({ id: s.id, title: s.title || s.id })) }); }}>Open room</button>
        {to && <button title={`Attach these to “${to.title}” and go back to it`} onClick={() => { done(); void A.attachTo(to.id, l); }}>Attach to chat</button>}
        <button title="Unpick everything" onClick={done}>Clear</button>
      </span>
    </div>
  );
}
