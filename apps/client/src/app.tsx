import { RunsOn } from "./runs/RunsOn";
import { CLIENT_VERSION } from "./version";
import { useEffect, useRef, useState } from "preact/hooks";
import { Component, Fragment } from "preact";
import { effect } from "@preact/signals";
import { render as md } from "./markdown";
import * as S from "./store";
import type { Entry, ThreadItem } from "./model";
import { Sidebar } from "./sidebar/Sidebar";
import { Home } from "./home/Home";
import { SpawnPanel } from "./panel/SpawnPanel";
import { Files, Images, Lightbox, ToolEntry } from "./transcript/Rich";
import { now, spoken, useTick } from "./home/util";
import { HeaderExtras, HeaderLinks, ActionDialogs } from "./actions/ThreadMenu";
import { Msg } from "./actions/MsgActions";
import * as A from "./actions/state";
import { DigestBlock } from "./digest/Digest";
import "./digest/state";
import { RichComposer as Composer } from "./transcript/Composer";
import { Replies } from "./transcript/Replies";
import { actionBlocks, replyBlocks } from "./transcript/suggested";
import { Actions } from "./transcript/Actions";
import { ScopeChips } from "./transcript/ScopePicker";
import { chipsOf } from "./scopes";
import * as O from "./transcript/outbox";
import { attention } from "./home/state";
import { loadRunners, runners, unreached } from "./runs/state";
import * as Nav from "./nav";
import { MapView, HomeSwitch } from "./map/MapView";
import * as MapState from "./map/state";
import { sideCollapsed } from "./sidebar/width";
import { zoomed } from "./transcript/Rich";
import { NewRoomDialog, NoteLine, RoomAsks, RoomBack, RoomTag, RosterBar, SpeakerHead, currentAsks } from "./room/Room";
import { speakerColor, waitingLine } from "./room/logic";
import { newRoom, openNewRoom } from "./room/state";

const showNew = A.showNew;
// Any navigation (home, a thread, back) leaves the new-thread form and a
// pending new thread's view: a later "+" or home must not come back to it.
effect(() => { void S.nav.value; showNew.value = false; S.creating.value = null; });

function PlacePicker() {
  useEffect(() => { void S.loadPlaces(); }, []);
  return (
    <label class="placepick" title="Where the new thread lives">
      <span class="meta">in</span>
      <select value={S.newPlace.value ?? ""} onChange={(e) => S.setNewPlace(e.currentTarget.value || null)}>
        <option value="">global (no folder)</option>
        {S.places.value.map((p) => <option key={p.key} value={p.key}>{p.label}</option>)}
      </select>
    </label>
  );
}

/** Where the new thread's turns run: this node, or another that runs
 * programs (design §2.7, "A new chat on another node"). */
function NodePicker() {
  useEffect(() => { void loadRunners(); }, []);
  const choices = runners.value.filter((r) => r.runs !== false);
  if (!choices.length) return null;
  return (
    <label class="placepick" title="Where the new thread's turns run">
      <span class="meta">on</span>
      <select value={S.newNode.value ?? ""} onChange={(e) => (S.newNode.value = e.currentTarget.value || null)}>
        <option value="">{attention.value?.node ?? "this node"} (here)</option>
        {choices.map((r) => <option key={r.name} value={r.name}>{r.name}{unreached(r)}</option>)}
      </select>
    </label>
  );
}

function Banner() {
  const s = S.connState.value;
  if (s === "open") return null;
  if (s.startsWith("mismatch:")) {
    return (
      <div class="banner">
        This page's client ({CLIENT_VERSION}) doesn't match the node ({s.slice("mismatch:".length)}):{" "}
        <a href="" onClick={() => { try { sessionStorage.removeItem("stitch-reloaded-for"); } catch { /* */ } }}>reload</a>
      </div>
    );
  }
  return <div class="banner">{s === "connecting" ? "Connecting to the node…" : "Node restarting — reconnecting…"}</div>;
}

function Md({ text }: { text: string }) {
  return <div class="md" dangerouslySetInnerHTML={{ __html: md(text) }} />;
}

/** A message said in a voice call (design §2.26): a small microphone by it. */
function Said() {
  return (
    <span class="said" title="Said in a voice call (an automatic transcription)" aria-label="Said in a voice call">
      <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><rect x="5.5" y="1" width="5" height="9" rx="2.5" fill="currentColor" /><path d="M3 7.5a5 5 0 0 0 10 0M8 12.5V15" stroke="currentColor" stroke-width="1.5" fill="none" stroke-linecap="round" /></svg>
    </span>
  );
}

/** Whether an entry shows the same: the same object (the stream keeps the
 * ones it did not change), or one read again that says the same. */
function sameEntry(a: Entry, b: Entry) {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}

/** One transcript entry, rendered again only when it changes: a turn's
 * stream, a list push or a read of the transcript re-renders the view, and
 * a long conversation's every message with it (its markdown, its tools). The
 * signals Msg and Item read re-render them on their own. */
class Row extends Component<{ e: Entry }> {
  override shouldComponentUpdate(next: { e: Entry }) {
    return !sameEntry(this.props.e, next.e);
  }
  override render() {
    const e = this.props.e;
    return <Msg e={e}><Item e={e} /></Msg>;
  }
}

/** Stable keys for a transcript's entries (by event, not by position:
 * a window read before them moves every index). */
function entryKeys(items: Entry[]): string[] {
  const seen = new Map<string, number>();
  return items.map((e, i) => {
    if (!("n" in e) || e.n < 0) return `i${i - items.length}`;
    const k = `${e.n}${e.kind}`;
    const dup = seen.get(k) ?? 0;
    seen.set(k, dup + 1);
    return dup ? `${k}.${dup}` : k;
  });
}

/** "Show earlier": the window before the transcript's, read keeping what
 * is on screen where it is. */
function Earlier({ box }: { box: { current: HTMLDivElement | null } }) {
  const w = S.transcriptWindow.value;
  if (!w?.earlier) return null;
  const more = async () => {
    const el = box.current;
    const from = el ? el.scrollHeight - el.scrollTop : 0;
    if ((await S.loadEarlier()) && el) requestAnimationFrame(() => (el.scrollTop = el.scrollHeight - from));
  };
  return (
    <div class="earlier-more">
      <button class="link" disabled={S.loadingEarlier.value} onClick={() => void more()}>
        {S.loadingEarlier.value ? "Reading…" : "Show earlier"}
      </button>
    </div>
  );
}

function Item({ e }: { e: Entry }) {
  switch (e.kind) {
    case "user": {
      const line = e.pending ? O.said(e, !!S.room.value) : null;
      const cls = "msg user" + (e.pending && e.outgoing !== "running" ? " pending" : "") + (e.outgoing === "failed" ? " failed" : "") + (e.spoken ? " spoken" : "");
      return <div class={cls}><RoomTag e={e} />{e.spoken && <Said />}<ScopeChips scopes={e.scopes} />{e.text && <Md text={e.text} />}<Images images={e.images} /><Files files={e.files} />{line && <div class={"sending " + (e.outgoing ?? "")}>{line}</div>}</div>;
    }
    case "assistant": return <div class={"msg asst" + (e.speaker ? " spoke" : "")} style={e.speaker ? `--sc:${speakerColor(e.speaker.slug)}` : undefined}>{e.speaker && <SpeakerHead s={e.speaker} />}<Md text={e.text} /><Images images={e.images} /><Files files={e.files} /></div>;
    case "live-assistant":
      if (e.speaker && !e.text.trim()) return null;
      return <div class={"msg asst streaming" + (e.speaker ? " spoke" : "")} style={e.speaker ? `--sc:${speakerColor(e.speaker.slug)}` : undefined}>{e.speaker && <SpeakerHead s={e.speaker} />}<Md text={e.text} /></div>;
    case "note": return <NoteLine text={e.text} />;
    case "failed": return <div class="turn-failed" role="alert">{e.text}</div>;
    case "tool": return <ToolEntry e={e} />;
    case "compacted": return <Compacted e={e} />;
    // An action button pressed: its line, the call's output folded.
    case "press": return (
      <div class={"press" + (e.ok ? "" : " failed")}>
        {e.output.trim() ? <details><summary>{e.text}</summary><pre>{e.output}</pre></details> : <span>{e.text}</span>}
      </div>
    );
  }
}

/** Where the conversation was compacted (design §2.15): one conversation, a
 * divider; the summary it goes on from, folded; the earlier thread as it was. */
function Compacted({ e }: { e: Extract<Entry, { kind: "compacted" }> }) {
  return (
    <div class="compacted">
      <div class="cline"><span>{e.said}</span></div>
      <div class="cmore">
        {e.summary && <details><summary>Summary</summary><Md text={e.summary} /></details>}
        {e.from && <button class="link" title={e.from} onClick={() => void S.select(e.from, true)}>the earlier thread, as it was</button>}
      </div>
    </div>
  );
}

/** Reading an earlier part of a compacted conversation: where it goes on. */
function EarlierPart() {
  const to = S.latest.value;
  if (!to) return null;
  return (
    <div class="earlier">
      An earlier part of this conversation, as it was: it goes on in a newer thread.{" "}
      <button class="link" onClick={() => void S.select(to)}>Go to the latest</button>
    </div>
  );
}

function Approval({ t }: { t: ThreadItem }) {
  const q = t.question;
  if (!q) return null;
  return (
    <div class="approval">
      <div class="q">Allow this command?</div>
      <code>{q.command}</code>
      <div class="btns">
        <button class="primary" onClick={() => void S.answer(q.id, true)}>Allow</button>
        <button onClick={() => void S.answer(q.id, true, true)}>Always</button>
        <button class="danger" onClick={() => void S.answer(q.id, false)}>Decline</button>
      </div>
    </div>
  );
}

/** The end of the transcript while a turn runs: that it is alive, what it does, and how long. */
function Working({ t }: { t: ThreadItem }) {
  useEffect(() => useTick(), []);
  void now.value;
  const turn = S.turn.value;
  const status = S.turnStatus.value;
  if (t.question) return <div class="working ask"><span class="adot" /> Waiting for your approval — answer above the message box.</div>;
  // A room whose participant asks (design §2.27): who, not "working".
  const asking = t.room ? currentAsks() : [];
  if (asking.length) {
    const p = asking[0]!.participant;
    return <div class="working ask"><span class="who" style={`--sc:${speakerColor(p.slug)}`}><i class="sdot" /></span><span class="adot" /> {waitingLine(asking)} — answer above the message box.</div>;
  }
  if (t.state === "blocked") return <div class="working ask"><span class="adot" /> Blocked: waiting on you.</div>;
  // The node's word first (a turn running, or accepted and starting), then
  // what this client saw stream, then the list's state; and a message of
  // ours waiting for its turn.
  const waiting = O.awaiting(S.entries.value);
  // The node's status covers its own loops: a thread another node runs is
  // read from the list.
  const ownStatus = status && attention.value?.node === t.node;
  const running = !!turn || !!status?.running || (!ownStatus && t.state === "working");
  const queued = status?.queued ?? 0;
  if (!running && !queued && !waiting) return null;
  if (!running) {
    const sending = S.entries.value.some((x) => O.isLocal(x) && x.outgoing === "sending");
    return (
      <div class="working">
        <span class="dots"><i /><i /><i /></span>
        <span class="wl">{sending && !queued ? "Sending…" : S.room.value ? "Starting the room's round…" : "Starting the turn…"}</span>
      </div>
    );
  }
  const call = [...S.entries.value].reverse().find((x) => x.kind === "tool" && !x.result);
  const step = call && call.kind === "tool" && call.at
    ? { what: call.call, since: call.at }
    : t.in_flight?.since ? { what: t.in_flight.headline || t.in_flight.name, since: t.in_flight.since } : null;
  const label = step ? "Running" : turn?.phase === "writing" ? "Writing" : turn?.phase === "thinking" ? "Thinking" : "Working";
  // A room's round (design §2.27): who speaks, or the council at once.
  const room = S.room.value;
  const who = room ? S.speaking.value : null;
  return (
    <div class="working">
      <span class="dots"><i /><i /><i /></span>
      {who && <span class="who" style={`--sc:${speakerColor(who.slug)}`}><i class="sdot" />{who.name}</span>}
      {room && !who && <span class="wl">{room.mode === "parallel" ? "The council answers…" : "The room's round…"}</span>}
      {(!room || who) && <span class="wl">{label}{step ? "" : "…"}</span>}
      {step && <code class="wcall" title={step.what}>{step.what.slice(0, 90)}</code>}
      {step && <span class="el">{spoken(step.since)}</span>}
      {queued > 0 && <span class="el">· {queued === 1 ? "a message" : `${queued} messages`} queued after it</span>}
    </div>
  );
}

function ThreadView() {
  const t = S.current.value;
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const items = S.entries.value;
  const keys = entryKeys(items);
  useEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
    // Opening a thread scrolls to its end; a short one fits whole.
    if (el) S.atEnd.value = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  }, [items, t?.id, S.turn.value?.phase]);
  // A new thread is shown pending until the node lists it (it names it before its first step).
  if (showNew.value || (S.creating.value && !t))
    return (
      <main>
        <header>
          <h1>New thread</h1>
          <PlacePicker />
          <NodePicker />
        </header>
        {S.creating.value ? (
          <div class="transcript">
            <div class="msg user pending"><ScopeChips scopes={chipsOf(S.creating.value.scopes ?? [])} /><Md text={S.creating.value.text} /><Images images={S.creating.value.images.filter((f) => f.type.startsWith("image/"))} /><Files files={S.creating.value.images.filter((f) => !f.type.startsWith("image/"))} /><div class="sending">sending…</div></div>
            <div class="working"><span class="dots"><i /><i /><i /></span><span class="wl">Starting the thread…</span></div>
          </div>
        ) : <div class="empty">Say what you want done.</div>}
        {S.error.value && <div class="err" onClick={() => (S.error.value = null)}>{S.error.value}</div>}
        <Composer thread={null} placeholder="Message a new thread…" onSend={(x, im, sc) => void S.newThread(x, im, sc)} />
      </main>
    );
  if (S.homeView.value || !S.selected.value) return Nav.homeMode.value === "map" ? null : <Home lead={<HomeSwitch />} />;
  if (!t) {
    // A thread outside the list's window: its transcript shows while its
    // row is read (`Threads { thread }`).
    const gone = S.extra.value[S.selected.value] === null;
    return (
      <main>
        <header>
          {Nav.fromMap.value && <button class="backmap" title="Back to the map (Esc)" onClick={Nav.back}>← Map</button>}
          <div class="htitle"><h1>{gone ? "No such thread" : "Loading the thread…"}</h1></div>
          <div class="hctl"><span class="meta">{S.selected.value}</span></div>
        </header>
        <div class="transcript" ref={box}>
          {items.map((e, i) => <Row key={keys[i]} e={e} />)}
        </div>
        {S.error.value && <div class="err" onClick={() => (S.error.value = null)}>{S.error.value}</div>}
      </main>
    );
  }
  // Preset answers (design §2.30): live while no turn runs.
  const blocks = replyBlocks(items, !!S.turn.value || !!S.turnStatus.value?.running || t.state === "working");
  // Action buttons (step 2): the latest set live, whatever runs.
  const sets = actionBlocks(items);
  return (
    <main>
      <header>
        {Nav.fromMap.value && <button class="backmap" title="Back to the map (Esc)" onClick={Nav.back}>← Map</button>}
        <div class="htitle"><h1>{t.title}</h1><HeaderLinks t={t} /></div>
        <div class="hctl">
          <span class="meta">{t.place} · {t.node} · {t.state}{t.cost != null ? ` · $${t.cost.toFixed(2)}` : ""}</span>
          <RunsOn t={t} />
          {(t.state === "working" || S.turn.value || S.turnStatus.value?.running || S.room.value?.running) && <button class="danger" title={S.room.value ? "Stop the room's round: every participant speaking now" : undefined} onClick={() => void S.cancel()}>{S.room.value ? "Stop round" : "Cancel"}</button>}
          <HeaderExtras t={t} />
        </div>
      </header>
      {t.room && <RosterBar />}
      <RoomBack t={t} items={items} />
      <EarlierPart />
      <div class="transcript" ref={box} onScroll={(e) => { const el = e.currentTarget; stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80; if (S.atEnd.value !== stick.current) S.atEnd.value = stick.current; }}>
        <DigestBlock key={t.id} thread={t.id} />
        <Earlier box={box} />
        {items.map((e, i) => {
          const b = blocks.get(i);
          const a = sets.get(i);
          // The call itself is not shown: its buttons stand for it.
          const call = e.kind === "tool" && (!!e.replies?.length || (!!e.actions?.length && e.result?.ok !== false));
          return <Fragment key={keys[i]}>{!call && <Row e={e} />}{a && <Actions b={a} thread={t.id} />}{b && <Replies b={b} onPick={(text) => void S.send(text)} />}</Fragment>;
        })}
        <Working t={t} />
      </div>
      <Approval t={t} />
      {t.room && <RoomAsks />}
      <OfflineOwner />
      {S.error.value && <div class="err" onClick={() => (S.error.value = null)}>{S.error.value}</div>}
      <Composer thread={t.id} room={t.room ? S.room.value?.participants ?? t.participants ?? [] : null}
        placeholder={t.room ? (t.state === "working" ? "Message the room (taken after the current speaker)…" : "Message the room…") : t.state === "working" ? "Interject into the running turn…" : "Message…"}
        onSend={(x, im, sc) => void S.send(x, im, false, sc)} />
    </main>
  );
}

/** The thread's node is offline: say so, and propose another node. */
function OfflineOwner() {
  const o = S.offlineOwner.value;
  if (!o) return null;
  const others = runners.value.filter((r) => r.runs === true && r.name !== o.node);
  const here = attention.value?.node;
  return (
    <div class="err offline">
      <div>{o.node} owns this thread and is offline. Continue it on another node?</div>
      <div class="meta">“{o.text.length > 120 ? o.text.slice(0, 120) + "…" : o.text}”</div>
      {others.map((r) => <button key={r.name} onClick={() => void S.resolveOffline(r.name)}>Continue on {r.name}</button>)}
      {here && <button onClick={() => void S.resolveOffline(null)}>Run here ({here})</button>}
      <button onClick={() => void S.resolveOffline("drop")}>Discard message</button>
    </div>
  );
}

/** Esc in a conversation opened from the map goes back to it, unless a layer of the conversation is open. */
function useBackOnEsc() {
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || !Nav.fromMap.peek() || S.homeView.peek() || showNew.peek()) return;
      if (A.dialog.peek() || A.menuOpen.peek() || zoomed.peek() || newRoom.peek()) return;
      if ((e.target as HTMLElement | null)?.closest?.("input, textarea, select, [contenteditable]")) return;
      Nav.back();
    };
    document.addEventListener("keydown", k);
    return () => document.removeEventListener("keydown", k);
  }, []);
}

export function App() {
  useBackOnEsc();
  const atMap = Nav.mapShown.value && !showNew.value;
  const home = S.homeView.value || !S.selected.value;
  return (
    <>
      <Banner />
      <div class={"layout " + (home ? "at-home" : "at-thread") + (atMap ? " at-map" : "") + (atMap && sideCollapsed.value ? " side-off" : "")}>
        <Sidebar onNew={() => { showNew.value = true; }} onNewRoom={() => openNewRoom()} />
        <ThreadView />
        {(MapState.mounted.value || atMap) && <MapView on={atMap} />}
        {!S.homeView.value && !showNew.value && <SpawnPanel />}
      </div>
      <Lightbox />
      <ActionDialogs />
      <NewRoomDialog />
    </>
  );
}
