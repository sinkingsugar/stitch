// Rooms in the web client (design §2.27, rooms.md §5): the roster bar at the
// top of a room, speaker headers on its replies, its quiet notes, the way
// back from a participant's own thread, and the New Room dialog (`+ Room`,
// "Pull in model…", the map's "Open room").
import "./room.css";
import { useEffect, useState } from "preact/hooks";
import * as S from "../store";
import * as A from "../actions/state";
import { attention, questions } from "../home/state";
import { loadRunners, runners, unreached } from "../runs/state";
import { StagedScopes, ScopePicker } from "../transcript/ScopePicker";
import { toggleScope, type StagedScope } from "../scopes";
import type { Entry, ThreadItem } from "../model";
import type { RoomInfo, RoomParticipant, Speaker } from "../payload";
import { modelSpec, participantCount, pillMeta, quietNote, roomAsks, roomNote, rowOf, secondOpinions, shortModel, speakerColor, threadSpec, type ModelPick, type RoomAsk } from "./logic";
import { create, mention, newRoom, placeKey, stopOne, update, type RoomSeed } from "./state";

const allRows = () => [...S.threads.value, ...Object.values(S.placeRows.value).flat()];
const titleOf = (id: string) => allRows().find((r) => r.id === id || r.earlier?.includes(id))?.title ?? id;

let modelsCache: A.ModelList | null = null;
/** The models the node offers (`Models`), read once. */
function useModels(): A.ModelList | null {
  const [m, setM] = useState<A.ModelList | null>(modelsCache);
  useEffect(() => { if (!modelsCache) A.models().then((x) => setM((modelsCache = x)), () => setM({ models: [], problems: [], default: "" })); }, []);
  return m;
}

// ---- speakers and notes in the transcript

/** A reply's header in a room: who said it, with its model and its own thread. */
export function SpeakerHead({ s }: { s: Speaker }) {
  return (
    <div class="speaker" style={`--sc:${speakerColor(s.slug)}`}>
      <i class="sdot" aria-hidden="true" />
      <b class="sname">{s.name}</b>
      <span class="sslug">@{s.slug}</span>
      {s.model && <span class="smodel" title={s.model}>· {shortModel(s.model)}</span>}
      {s.thread && <button class="sthread" title="Open its own thread: its tool calls and thinking" onClick={() => void S.select(s.thread)}>thread ↗</button>}
    </div>
  );
}

/** A note of the transcript; a room's (a pass, a stop) quiet, with its speaker's colour. */
export function NoteLine({ text }: { text: string }) {
  const r = S.room.value;
  const n = r ? roomNote(text, r.participants) : null;
  if (!n) return <div class="note">{text}</div>;
  const slug = n.kind === "speaker" ? n.speaker.slug : n.kind === "room" ? null : n.slug;
  return (
    <div class={"note rnote " + n.kind} style={slug ? `--sc:${speakerColor(slug)}` : undefined}>
      {slug && <i class="sdot" aria-hidden="true" />}{quietNote(n)}
    </div>
  );
}

/** On a user message of a room: the seed of "Pull in model…", or a
 * participant's relayed room messages (its own thread). */
export function RoomTag({ e }: { e: Extract<Entry, { kind: "user" | "assistant" }> }) {
  if (e.from === "room:seed") return <div class="roomtag-msg">⇄ how the room began: what the newcomers were briefed with</div>;
  if (e.room_turn) {
    const rt = e.room_turn;
    return <div class="roomtag-msg"><button class="link" title={`Relayed from the room up to ${rt.room}#${rt.upto}`} onClick={() => void S.select(rt.room)}>⇄ relayed from the room</button> <span class="meta">as @{rt.slug}</span></div>;
  }
  return null;
}

/** A participant's own thread: the way back to its room. */
export function RoomBack({ t, items }: { t: ThreadItem; items: Entry[] }) {
  if (t.room) return null;
  const parent = t.parent && allRows().find((r) => r.id === t.parent && r.room);
  let room: string | null = parent ? parent.id : null;
  if (!room) for (let i = items.length - 1; i >= 0 && !room; i--) { const e = items[i]!; if (e.kind === "user" && e.room_turn) room = e.room_turn.room; }
  if (!room) return null;
  const go = room;
  return (
    <div class="roomback">
      This thread speaks in a room: what it was relayed and its own tool calls are here.{" "}
      <button class="link" onClick={() => void S.select(go)}>⇄ Back to the room “{titleOf(go)}”</button>
    </div>
  );
}

// ---- participants' questions (design §2.27)

/** The questions the open room's participants wait on (their rows', else
 * the attention list's). */
export function currentAsks(): RoomAsk[] {
  const r = S.room.value;
  const t = S.current.value;
  const roster = r?.participants ?? t?.participants ?? [];
  if (!t?.room || !roster.length) return [];
  return roomAsks(roster, allRows(), questions.value.map((q) => ({ id: q.id, command: q.command ?? "", thread: q.thread, grant: q.grant })));
}

/** A participant's question, in the room: who asks (as its replies' header
 * says it), what, and Allow / Always / Decline (with a reason). */
function AskCard({ a }: { a: RoomAsk }) {
  const [why, setWhy] = useState("");
  const p = a.participant;
  const always = a.question.grant?.length ? `Always: ${a.question.grant.join(", ")}` : `Always allow this for ${p.name}`;
  return (
    <div class="approval rask" style={`--sc:${speakerColor(p.slug)}`}>
      <div class="speaker">
        <i class="sdot" aria-hidden="true" />
        <b class="sname">{p.name}</b>
        <span class="sslug">@{p.slug}</span>
        {p.model && <span class="smodel" title={p.model}>· {shortModel(p.model)}</span>}
        <button class="sthread" title="Open its own thread: its tool calls and thinking" onClick={() => void S.select(a.question.thread ?? p.thread)}>thread ↗</button>
      </div>
      <div class="q">{a.text}</div>
      {a.what && <code>{a.what}</code>}
      <div class="btns">
        <button class="primary" onClick={() => void S.answer(a.question.id, true)}>Allow</button>
        <button title={always} onClick={() => void S.answer(a.question.id, true, true)}>Always</button>
        <input class="rwhy" placeholder="Why not (optional)" aria-label={`Why ${p.name} may not`} value={why} onInput={(e) => setWhy(e.currentTarget.value)} />
        <button class="danger" onClick={() => void S.answer(a.question.id, false, false, why.trim() || null)}>Decline</button>
      </div>
    </div>
  );
}

/** The room's participants' questions, above the message box. */
export function RoomAsks() {
  const asks = currentAsks();
  if (!asks.length) return null;
  return <>{asks.map((a) => <AskCard key={a.question.id} a={a} />)}</>;
}

// ---- the roster bar

function Pill({ r, p }: { r: RoomInfo; p: RoomParticipant }) {
  const row = rowOf(p, allRows());
  const here = attention.value?.node ?? null;
  const speaking = S.speaking.value?.slug === p.slug;
  const working = speaking || row?.state === "working";
  const meta = pillMeta(p, row, here);
  const asks = currentAsks().some((a) => a.participant.slug === p.slug);
  return (
    <span class={"pill" + (p.muted ? " muted" : "") + (speaking ? " speaking" : "") + (asks ? " asks" : "")} style={`--sc:${speakerColor(p.slug)}`}>
      <i class="sdot" aria-hidden="true" />
      {asks && <span class="adot" title={`${p.name} is waiting for your approval`} aria-label={`${p.name} is waiting for your approval`} />}
      <button class="pname" title={`Open ${p.name}'s own thread (${p.model ?? "its model"}): its tool calls and thinking`} onClick={() => void S.select(p.thread)}>{p.name}</button>
      <button class="pslug" title={`Address @${p.slug} in your message`} onClick={() => (mention.value = p.slug)}>@{p.slug}</button>
      {meta && <span class="pmeta" title={p.model ?? ""}>{meta}</span>}
      {row && (row.read_only
        ? <button class="pro" title={`Read-only: it asks you before it changes files. Make ${p.name} writable`} aria-label={`Make ${p.name} writable`} onClick={() => void A.readOnly(row.id, false)}>ro</button>
        : <button class="pro rw" title={`Writable. Make ${p.name} read-only (it asks before writing)`} aria-label={`Make ${p.name} read-only`} onClick={() => void A.readOnly(row.id, true)}>rw</button>)}
      {working && <button class="pstop" title={`Stop ${p.name}'s turn (the round goes on)`} aria-label={`Stop ${p.name}`} onClick={() => void stopOne(p.thread)}>■</button>}
      <button class="pmute" aria-pressed={p.muted} title={p.muted ? "Unmute: it answers again" : "Mute: it answers only when @mentioned"} onClick={() => void update(r.room, p.muted ? { unmute: [p.slug] } : { mute: [p.slug] })}>{p.muted ? "🔇" : "🔈"}</button>
      <button class="prm" title={`Remove ${p.name} from the room (its thread stays)`} aria-label={`Remove ${p.name}`} onClick={() => { if (confirm(`Remove ${p.name} (@${p.slug}) from the room? Its thread stays.`)) void update(r.room, { remove: [p.slug] }); }}>×</button>
    </span>
  );
}

/** Picks a model: a list, how it may act, and where it runs. */
function ModelAdder({ onAdd, exclude = [], label = "Add" }: { onAdd: (p: ModelPick) => void; exclude?: string[]; label?: string }) {
  const m = useModels();
  useEffect(() => { if (!runners.value.length) void loadRunners(); }, []);
  const [model, setModel] = useState("");
  const [access, setAccess] = useState<ModelPick["access"]>("read-only");
  const [node, setNode] = useState("");
  const list = (m?.models ?? []).filter((x) => !exclude.includes(x));
  const chosen = model || list[0] || "";
  const nodes = runners.value.filter((r) => r.runs !== false);
  return (
    <div class="madder">
      <select aria-label="Model" value={chosen} onChange={(e) => setModel(e.currentTarget.value)}>
        {list.map((x) => <option key={x} value={x}>{x}{x === m?.default ? " (default)" : ""}</option>)}
        <option value="default">default (the synced default)</option>
        <option value="quick">quick (the quick model)</option>
      </select>
      <select aria-label="What it may do" value={access} onChange={(e) => setAccess(e.currentTarget.value as ModelPick["access"])}>
        <option value="read-only" title="Reads and searches; a write or a command that changes something asks you first">read-only (asks before writing)</option>
        <option value="writable" title="Works as any thread of the place, under its rules">can write</option>
        <option value="no-tools" title="An opinion only">no tools</option>
      </select>
      {nodes.length > 0 && (
        <select aria-label="Where it runs" value={node} onChange={(e) => setNode(e.currentTarget.value)}>
          <option value="">here</option>
          {nodes.map((r) => <option key={r.name} value={r.name}>@{r.name}{unreached(r)}</option>)}
        </select>
      )}
      <button disabled={!chosen} onClick={() => onAdd({ model: chosen, access, node: node || null })}>{label}</button>
      {!m && <span class="meta">loading models…</span>}
    </div>
  );
}

/** Finds a thread to invite (the list's, rooms left out). */
function ThreadFinder({ onPick, exclude }: { onPick: (t: ThreadItem) => void; exclude: string[] }) {
  const [q, setQ] = useState("");
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const list = allRows()
    .filter((t) => !t.room && !exclude.includes(t.id) && words.every((w) => `${t.title}\n${t.place}\n${t.id}`.toLowerCase().includes(w)))
    .slice(0, 6);
  return (
    <div class="tfinder">
      <input type="search" placeholder="Invite a thread: search by title, place or ID" value={q} onInput={(e) => setQ(e.currentTarget.value)} />
      {q && list.map((t) => (
        <button key={t.id} class="mi" title={t.id} onClick={() => { onPick(t); setQ(""); }}>
          + {t.title} <span class="meta">{t.place} · {t.node}</span>
        </button>
      ))}
      {q && !list.length && <div class="meta">No thread matches.</div>}
    </div>
  );
}

function AddMenu({ r }: { r: RoomInfo }) {
  const [open, setOpen] = useState(false);
  const inRoom = r.participants.map((p) => p.thread);
  return (
    <div class="menu raddmenu">
      <button class="radd" title="Add a model or invite a thread" aria-expanded={open} onClick={() => setOpen(!open)}>+ Add</button>
      {open && (
        <>
          <div class="scrim" onClick={() => setOpen(false)} />
          <div class="mlist radd-pop" onKeyDown={(e) => { if (e.key === "Escape") setOpen(false); }}>
            <h4>A model</h4>
            <ModelAdder onAdd={(p) => { setOpen(false); void update(r.room, { add: [modelSpec(p)] }); }} />
            <h4>A thread, as it is</h4>
            <ThreadFinder exclude={[r.room, ...inRoom]} onPick={(t) => { setOpen(false); void update(r.room, { add: [threadSpec(t.id)] }); }} />
          </div>
        </>
      )}
    </div>
  );
}

/** At the top of a room: who is in it, how its rounds go. */
export function RosterBar() {
  const r = S.room.value;
  if (!r) return null;
  const from = r.promoted_from;
  return (
    <div class="roster" role="region" aria-label="The room's participants">
      <div class="pills">
        {r.participants.map((p) => <Pill key={p.slug} r={r} p={p} />)}
        <AddMenu r={r} />
      </div>
      <div class="policy">
        <div class="seg" role="group" aria-label="Turn policy">
          <button class={r.mode === "sequential" ? "on" : ""} aria-pressed={r.mode === "sequential"} title="One at a time: each hears the earlier ones, may pass, and may hand off with @" onClick={() => r.mode !== "sequential" && void update(r.room, { mode: "sequential" })}>Sequential</button>
          <button class={r.mode === "parallel" ? "on" : ""} aria-pressed={r.mode === "parallel"} title="Council: everyone at once, blind to each other until the next round" onClick={() => r.mode !== "parallel" && void update(r.room, { mode: "parallel" })}>Council</button>
        </div>
        {r.mode === "sequential" && (
          <label class="hops" title="How many times participants may hand off to each other (@name) per message of yours">
            hand-offs
            <select value={r.hop_cap} onChange={(e) => void update(r.room, { hopCap: +e.currentTarget.value })}>
              {[0, 1, 2, 3, 4].map((n) => <option key={n} value={n}>{n}</option>)}
            </select>
          </label>
        )}
        {r.budget_usd != null && <span class="meta">budget ${r.budget_usd.toFixed(2)}</span>}
        {from && <button class="link rfrom" title={from} onClick={() => void S.select(from)}>⇠ from “{titleOf(from).slice(0, 36)}”</button>}
      </div>
    </div>
  );
}

// ---- the New Room dialog

export function NewRoomDialog() {
  const seed = newRoom.value;
  if (!seed) return null;
  return <NewRoomForm seed={seed} />;
}

function NewRoomForm({ seed }: { seed: RoomSeed }) {
  const m = useModels();
  const [pulled, setPulled] = useState(!!seed.from);
  const from = seed.from ?? (pulled ? seed.pullable ?? null : null);
  const [picks, setPicks] = useState<ModelPick[] | null>(null);
  const [invited, setInvited] = useState<string[]>(seed.invite ?? []);
  const [extra, setExtra] = useState<{ id: string; title: string }[]>([]);
  const [scopes, setScopes] = useState<StagedScope[]>(seed.scopes ?? []);
  const [mode, setMode] = useState<"sequential" | "parallel">("sequential");
  const [hop, setHop] = useState(2);
  const [title, setTitle] = useState("");
  const [place, setPlace] = useState<string | null | undefined>(undefined);
  const [budget, setBudget] = useState("");
  const close = () => (newRoom.value = null);
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && close();
    addEventListener("keydown", k);
    return () => removeEventListener("keydown", k);
  }, []);
  // Second opinions: another provider than the thread's model (one when
  // pulling in, two for a new room); read-only.
  useEffect(() => {
    if (!m || picks) return;
    const seedPicks = (cur: string | null) => setPicks(secondOpinions(cur, m.models, from ? 1 : 2).map((model) => ({ model, access: "read-only" })));
    if (from) A.effectiveModel(from.id, null).then(seedPicks, () => seedPicks(null));
    else seedPicks(null);
  }, [m]);
  const key = place !== undefined ? place : seed.place != null ? placeKey(seed.place) : S.newPlace.value;
  const offered = [...(seed.offer ?? []), ...extra].filter((t) => t.id !== from?.id);
  const models = picks ?? [];
  // Pulled in, the thread briefs the newcomers itself: its own scope is not asked again.
  const spec = { from: from?.id ?? null, models, threads: invited.filter((x) => x !== from?.id), scopes: from ? scopes.filter((s) => s.id !== from.id) : scopes, mode, hopCap: hop, title, place: key, budgetUsd: budget.trim() ? +budget : null };
  const n = participantCount(spec);
  const go = () => void create(spec);
  return (
    <div class="modal" onClick={close}>
      <div class="card roomdlg" role="dialog" aria-label={from ? "Pull in a model" : "New room"} onClick={(e) => e.stopPropagation()}>
        <h3>{from ? "Pull in a model" : "New room"}</h3>
        {!seed.from && seed.pullable && (
          <div class="seg" role="group" aria-label="How the room starts">
            <button class={!pulled ? "on" : ""} aria-pressed={!pulled} title="A new room whose participants are briefed on it" onClick={() => setPulled(false)}>A room about it</button>
            <button class={pulled ? "on" : ""} aria-pressed={pulled} title="The conversation itself joins first and goes on in the room" onClick={() => setPulled(true)}>Pull in a model</button>
          </div>
        )}
        {from ? (
          <div class="meta">A room in this conversation's place: <b>“{from.title}”</b> joins first, as it is (its history and cache untouched), and the models below are briefed on it.</div>
        ) : (
          <div class="meta">Models and threads talking together. Each participant is a thread of its own; a model joins read-only unless you say otherwise.</div>
        )}
        <h4>Models</h4>
        <div class="plist">
          {models.map((p, i) => (
            <div key={p.model + i} class="prow">
              <span class="pm">{p.model}{p.node ? <span class="meta"> @{p.node}</span> : null}</span>
              <select aria-label="What it may do" value={p.access} onChange={(e) => setPicks(models.map((x, j) => (j === i ? { ...x, access: e.currentTarget.value as ModelPick["access"] } : x)))}>
                <option value="read-only" title="Reads and searches; a write or a command that changes something asks you first">read-only (asks before writing)</option>
                <option value="writable" title="Works as any thread of the place, under its rules">can write</option>
                <option value="no-tools" title="An opinion only">no tools</option>
              </select>
              <button class="prm" aria-label={`Remove ${p.model}`} onClick={() => setPicks(models.filter((_, j) => j !== i))}>×</button>
            </div>
          ))}
          {!picks && <div class="meta">Loading models…</div>}
          <ModelAdder label="+ Model" onAdd={(p) => setPicks([...models, p])} />
        </div>
        <h4>Threads</h4>
        <div class="plist">
          {from && <label class="prow"><input type="checkbox" checked disabled /> <span class="pm">{from.title}</span><span class="meta">the conversation pulled from</span></label>}
          {offered.map((t) => (
            <label key={t.id} class="prow">
              <input type="checkbox" checked={invited.includes(t.id)} onChange={(e) => setInvited(e.currentTarget.checked ? [...invited, t.id] : invited.filter((x) => x !== t.id))} />
              <span class="pm">{t.title}</span><span class="meta">joins as it is</span>
            </label>
          ))}
          <ThreadFinder exclude={[...(from ? [from.id] : []), ...offered.map((t) => t.id)]} onPick={(t) => { setExtra([...extra, { id: t.id, title: t.title }]); setInvited([...invited, t.id]); }} />
        </div>
        <h4>Attached</h4>
        <div class="rscopes">
          <StagedScopes scopes={scopes} onRemove={(s) => setScopes(scopes.filter((x) => x !== s))} />
          <ScopePicker scopes={scopes} onToggle={(s) => setScopes(toggleScope(scopes, s))} />
          {!scopes.length && <span class="meta">Threads or efforts to brief everyone on (⎇)</span>}
        </div>
        <div class="frm">
          <div class="seg" role="group" aria-label="Turn policy">
            <button class={mode === "sequential" ? "on" : ""} aria-pressed={mode === "sequential"} onClick={() => setMode("sequential")}>Sequential</button>
            <button class={mode === "parallel" ? "on" : ""} aria-pressed={mode === "parallel"} onClick={() => setMode("parallel")}>Council</button>
            {mode === "sequential" && (
              <label class="hops">hand-offs <select value={hop} onChange={(e) => setHop(+e.currentTarget.value)}>{[0, 1, 2, 3, 4].map((x) => <option key={x} value={x}>{x}</option>)}</select></label>
            )}
          </div>
          <div class="meta">{mode === "sequential" ? "One at a time: each hears the earlier ones and may pass; @name hands off." : "Everyone answers at once, blind to the others until the next round."}</div>
          {!from && (
            <label>Place
              <select value={key ?? ""} onChange={(e) => setPlace(e.currentTarget.value || null)}>
                <option value="">global (no folder)</option>
                {S.places.value.map((p) => <option key={p.key} value={p.key}>{p.label}</option>)}
              </select>
            </label>
          )}
          <label>Title <input placeholder={from ? `${from.title} · room` : "from its first exchange"} value={title} onInput={(e) => setTitle(e.currentTarget.value)} /></label>
          <label>Budget <input class="num" type="number" min={0} step={0.5} placeholder="none" value={budget} onInput={(e) => setBudget(e.currentTarget.value)} /> <span class="meta">dollars, all participants together</span></label>
        </div>
        <div class="btns end">
          <span class="meta">{n} participant{n === 1 ? "" : "s"}</span>
          <button onClick={close}>Cancel</button>
          <button class="primary" disabled={n < 2 || (!models.length && !invited.length)} onClick={go}>{from ? "Pull in & open" : "Create & open"}</button>
        </div>
      </div>
    </div>
  );
}
