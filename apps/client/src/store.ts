// Application state as signals, and the actions the views call.
import { signal, computed, effect } from "@preact/signals";
import { LIST_LIMIT, type Api, type ConnState } from "./conn";
import type { Request } from "./api/Request";
import type { Reply } from "./api/Reply";
import { attention, initLive, refreshLive, setSection } from "./home/state";
import * as D from "./deltas";
import { homeView, threads } from "./home/source";
import { coalesced, debounced } from "./coalesce";
import { initRuns, replyEvent, loadRuns, runOn, runners, pinned } from "./runs/state";
import type { Entry, LiveEvent, ThreadItem, TurnStatus } from "./model";
import type { Compaction } from "./api/Compaction";
import type { AppInfo } from "./api/AppInfo";
import type { PlaceCount, RoomInfo, Speaker } from "./payload";
import { roomNote } from "./room/logic";
import * as O from "./transcript/outbox";
import { readTarget, readThrough, unreadOf } from "./readstate";
import { attachmentArg, type Staged } from "./transcript/blob";
import { chipsOf, scopeTargets, type StagedScope } from "./scopes";
import { setAppOpener, setAppPins } from "./transcript/Rich";
import * as Nav from "./nav";
import type { Route } from "./route";

export const connState = signal<ConnState>("connecting");
/** The thread list and the home view's flag live with the home view's source
 * (home/source.ts), which the dashboard shares. */
export { threads, homeView };
export { setHomeMode } from "./nav";
export const selected = signal<string | null>(null);
/** The conversation opened last, kept when going home (the map's "Attach to chat"). */
export const lastOpen = signal<string | null>(null);
export const entries = signal<Entry[]>([]);
/** How many entries a conversation opens with (its end); "Show earlier"
 * reads as many again. A compacted conversation of thousands of entries
 * (tool output included) was megabytes to read and render whole. */
const WINDOW = 300;
/** The open transcript's window: the `n` of its first entry, and whether
 * entries precede it. Null: not read yet. */
export const transcriptWindow = signal<{ start: number; earlier: boolean } | null>(null);
export const loadingEarlier = signal(false);
/** The thread the open one is a fork of (`Transcript.forked_from`). */
export const forkedFrom = signal<{ thread: string; title: string | null } | null>(null);
export const error = signal<string | null>(null);
export const filter = signal("");
/** Bumped by every navigation (select, home): a "new thread" form closes on it. */
export const nav = signal(0);
/** Bumped (debounced) when the node says threads changed: the map reads its forest again. */
export const changeTick = signal(0);
/** The rows a list delta moved (design §2.24): the map applies them to its forest. */
export const listDelta = signal<{ upsert: ThreadItem[]; remove: string[] } | null>(null);
/** The whole forest (`Threads { forest }`), for the map. */
export const loadForest = () => api.listForest();

/** The open thread's row when the list's window leaves it out (an older
 * thread opened from a search hit, a link): read on its own
 * (`Threads { thread }`) when opened and on each refresh, so a refresh does
 * not lose it. Null: the node knows no such thread; absent: being read. */
export const extra = signal<Record<string, ThreadItem | null>>({});

/** Set when the open thread is an earlier part of a compacted conversation,
 * opened as it was (the divider's link): the newest thread, where it goes on.
 * Null otherwise: a thread opened on an old ID is switched to the newest. */
export const latest = signal<string | null>(null);

/** The open thread's room (design §2.27): what its transcript's `room`
 * says, or a `RoomUpdate` answered since; null when it is no room. */
export const room = signal<RoomInfo | null>(null);
/** Who speaks in the open room's round now (its `[Name · model]` note). */
export const speaking = signal<Speaker | null>(null);

/** A room's provisional info from its list row (until its transcript says). */
const roomOfRow = (r: ThreadItem): RoomInfo | null =>
  r.room ? { room: r.id, title: r.title, promoted_from: null, budget_usd: null, mode: r.room_mode ?? "sequential", hop_cap: 2, participants: r.participants ?? [], running: r.state === "working" } : null;

/** The open thread's row: a compaction chain's row is its newest thread's,
 * which lists the earlier ones (`earlier`). */
export const current = computed(() => {
  const id = selected.value;
  if (!id) return null;
  return threads.value.find((t) => t.id === id || t.earlier?.includes(id)) ?? extra.value[id] ?? null;
});

/** What a transcript's messages read of the open row, as values that change
 * only when they do (`current` is a new row on every list push: every
 * message re-rendered on each, mid-turn). */
export const currentId = computed(() => current.value?.id ?? null);
export const currentWorking = computed(() => current.value?.state === "working");

/** Reads the open thread's row when the list does not have it. */
async function readExtra(id: string) {
  if (threads.value.some((t) => t.id === id || t.earlier?.includes(id))) return;
  try {
    const row = await api.thread(id);
    // Only the open thread's row is kept. A new thread the node has named
    // but not written yet (it is written with its first step, which can
    // take a minute of thinking) is shown from its provisional row.
    if (selected.value === id) extra.value = { [id]: row ?? provisional.get(id) ?? null };
  } catch (e) {
    error.value = String(e);
  }
}

/** What a running turn is doing, for the indicator at the transcript's end. */
export type Phase = "waiting" | "thinking" | "writing" | "tool";
/** The open thread's turn as this client follows it (`Started`..`Ended`). */
export const turn = signal<{ phase: Phase; since: number } | null>(null);
/** Where the open thread's turns stand on the node, as it says when followed
 * and on each change (none: not said yet, or a node from before it). */
export const turnStatus = signal<TurnStatus | null>(null);
/** A new thread being created: its first message, shown pending until the node names the thread. */
export const creating = signal<{ text: string; images: Staged[]; scopes?: StagedScope[] } | null>(null);
/** Where the next new thread goes (`PlaceInfo.key`; none: global). */
export const newPlace = signal<string | null>(localStorage.getItem("stitch.place"));
export const places = signal<{ key: string; label: string }[]>([]);
/** The node the next new thread's turns run on (`Chat.node`); none: this one. */
export const newNode = signal<string | null>(null);

/** Opens the "new thread" form's data: the places, and the default one
 * (the open thread's, else the last used). */
export async function loadPlaces() {
  try {
    const r = await api.live<{ places: { key: string; label: string }[] }>("places");
    places.value = r.places;
    const open = current.value;
    const mine = open && !homeView.value ? r.places.find((p) => p.label === open.place) : undefined;
    if (mine) newPlace.value = mine.key;
    else if (newPlace.value && !r.places.some((p) => p.key === newPlace.value)) newPlace.value = null;
  } catch (e) {
    error.value = String(e);
  }
}
export function setNewPlace(key: string | null) {
  newPlace.value = key;
  if (key) localStorage.setItem("stitch.place", key);
  else localStorage.removeItem("stitch.place");
}

let api: Api;
/** A request on the connection, for the thread actions (src/actions). */
export const rpc = (r: Request): Promise<Reply> => api.call(r);
/** The model picked for a thread's next turns (`Chat.model`); none: the node's choice. */
export const picks = signal<Record<string, string>>({});
/** Reads the open thread's transcript again (after a rewind). */
export const reloadOpen = () => (selected.value ? reload(selected.value) : Promise.resolve());
/** Closes the open thread (back to the home view). */
export function goHome(how: "push" | "replace" | "none" = "push") {
  Nav.homing(how);
  void flushRead();
  unfollow?.();
  stash(selected.value);
  selected.value = null;
  homeView.value = true;
  nav.value++;
}
let unfollow: (() => void) | null = null;

export function init(a: Api) {
  api = a;
  initLive(a, { open: (id) => void select(id), answer: (q, allow, always) => answer(q, allow, always) });
  initRuns(a);
  setAppOpener(async (what) => {
    const r = await a.call({ AppOpen: { what } });
    return typeof r === "object" && "App" in r ? r.App.url : null;
  });
  setAppPins({
    // This version, or the app pinned from the open thread under its name
    // that follows it (it moved on to a newer version since).
    pinned: (manifest, name) =>
      apps.value.find((p) => p.version === manifest || (!p.fixed && p.thread === selected.value && p.name === name))?.id ?? null,
    // By its version, following its thread (the manifest names the thread).
    pin: async (manifest) => {
      try {
        await a.call({ AppPin: { what: manifest, name: null, icon: null, fixed: false } });
      } catch (e) {
        error.value = `pin: ${e instanceof Error ? e.message : e}`;
      }
      await readApps();
    },
    unpin: unpinApp,
  });
  // Deltas pushed (the node's `watch`): the list comes whole on connecting,
  // then the rows that moved; the live sections as they move; the running
  // calls of the threads shown working and the open one.
  const deltas = !!(a.onList && a.onLive && a.onCall && a.watchCalls);
  if (deltas) {
    a.onList!(applyList);
    a.onLive!(setSection);
    a.onCall!((thread, f) => {
      threads.value = D.withCall(threads.value, thread, f);
      const e = extra.value[thread];
      if (e) extra.value = { ...extra.value, [thread]: D.withCall([e], thread, f)[0]! };
    });
    effect(() => {
      const want = threads.value.filter((t) => t.state === "working").map((t) => t.id);
      const open = selected.value;
      if (open && !want.includes(open)) want.push(open);
      a.watchCalls!(want);
    });
  }
  a.onState((s) => {
    connState.value = s;
    if (s === "open") {
      // Also after a reconnect: what happened meanwhile is read again (with
      // deltas, the node sends the list whole itself).
      if (!deltas) void refresh();
      void readApps();
      if (selected.value) void reload(selected.value);
    }
  });
  // The node pushes `changed` very often while threads work: one round after
  // things go quiet (500 ms, at most 2 s), itself one in flight and one queued.
  effect(() => { void current.value; void atEnd.value; void focused.value; void entries.value; void homeView.value; considerRead(); });
  a.onThreadsChanged(debounced(() => { void refresh(); void refreshLive(); changeTick.value++; }));
}

/** The list's version as last applied (`list` frames, `deltas.ts`). */
let listV = 0;
function applyList(p: D.ListPush) {
  const now: D.ListState = { v: listV, threads: threads.value, places: placeCounts.value, placeRows: placeRows.value, extra: extra.value };
  const next = D.apply(now, p, LIST_LIMIT);
  if (next === "resync") {
    api.watch?.();
    return;
  }
  listV = next.v;
  threads.value = next.threads;
  placeCounts.value = next.places;
  if (p.reset) {
    // The sections and the open thread's row, read again; the map too.
    void Promise.all(Object.keys(placeRows.value).map(readPlace));
    if (selected.value) void readExtra(selected.value);
    changeTick.value++;
  } else {
    placeRows.value = next.placeRows;
    extra.value = next.extra;
    listDelta.value = { upsert: p.upsert ?? [], remove: p.remove ?? [] };
  }
}

/** Every place of the listed threads and how many each has, as the node counts
 * them (the list's window may hold only some of a place's threads). */
export const placeCounts = signal<PlaceCount[]>([]);
/** The threads of places the window does not cover, read with `Threads { place }`
 * when the section was opened (the sidebar), and again on each refresh. */
export const placeRows = signal<Record<string, ThreadItem[]>>({});
/** The sections the user opened or closed (place → open), kept in this browser. */
export const placeOpen = signal<Record<string, boolean>>(loadPlaceOpen());
const PLACES_KEY = "stitch.places";
function loadPlaceOpen(): Record<string, boolean> {
  try {
    const v = JSON.parse(localStorage.getItem(PLACES_KEY) ?? "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}
export function setPlaceOpen(place: string, open: boolean) {
  placeOpen.value = { ...placeOpen.value, [place]: open };
  localStorage.setItem(PLACES_KEY, JSON.stringify(placeOpen.value));
  if (!open && place in placeRows.value) {
    const { [place]: _, ...rest } = placeRows.value;
    placeRows.value = rest;
  }
}
/** Closes every listed section at once (the sidebar's "collapse all"), the
 * pinned and the apps sections with them. */
export function collapseAll(places: string[]) {
  const next = { ...placeOpen.value };
  for (const p of places) next[p] = false;
  placeOpen.value = next;
  localStorage.setItem(PLACES_KEY, JSON.stringify(next));
  placeRows.value = {};
  setPinnedOpen(false);
  if (appsOpen.value) setAppsOpen(false);
}
const ORDER_KEY = "stitch.sideOrder";
/** How the sidebar groups the threads: by place (the default) or by day, as
 * the app's list does; kept in this browser. */
export const sideOrder = signal<"place" | "time">(typeof localStorage !== "undefined" && localStorage.getItem(ORDER_KEY) === "time" ? "time" : "place");
export function setSideOrder(order: "place" | "time") {
  sideOrder.value = order;
  localStorage.setItem(ORDER_KEY, order);
}
const PINNED_KEY = "stitch.pinned";
/** The Pinned section is open, kept in this browser (open by default). */
export const pinnedOpen = signal(typeof localStorage === "undefined" || localStorage.getItem(PINNED_KEY) !== "0");
export function setPinnedOpen(open: boolean) {
  pinnedOpen.value = open;
  localStorage.setItem(PINNED_KEY, open ? "1" : "0");
}
/** The pinned apps (`Request::Apps`, the built-in client left out), read on
 * connecting and each time the sidebar's Apps section is opened. */
export const apps = signal<AppInfo[]>([]);
const APPS_KEY = "stitch.apps";
/** The Apps section is open, kept in this browser (closed by default). */
export const appsOpen = signal(typeof localStorage !== "undefined" && localStorage.getItem(APPS_KEY) === "1");
export async function readApps() {
  try {
    const r = await api.call("Apps");
    if (typeof r === "object" && "Apps" in r) apps.value = [...r.Apps.apps].sort((a, b) => a.name.localeCompare(b.name));
  } catch {
    // An older node, or none reachable: the section stays as it was.
  }
}
export function setAppsOpen(open: boolean) {
  appsOpen.value = open;
  localStorage.setItem(APPS_KEY, open ? "1" : "0");
  if (open) void readApps();
}
/** Remove the pin by ID, without needing its thread or app version. */
export async function unpinApp(app: string) {
  try {
    await rpc({ AppUnpin: { app } });
    apps.value = apps.peek().filter((p) => p.id !== app);
  } catch (e) {
    error.value = `unpin: ${e instanceof Error ? e.message : e}`;
  }
  await readApps();
}
/** Opens a pinned app in a tab of its own, at the node's address with its token. */
export async function openApp(a: AppInfo) {
  // A tab opened before the await, so a popup blocker lets it through.
  const w = window.open("", "_blank");
  let url = a.url;
  if (!url) {
    const r = await api.call({ AppOpen: { what: a.id } }).catch(() => null);
    url = r && typeof r === "object" && "App" in r ? r.App.url : null;
  }
  if (url && w) w.location.href = url;
  else {
    w?.close();
    error.value = `${a.name}: this node serves no browser for apps`;
  }
}
/** Reads one place's threads (all of them, up to the node's limit). */
export async function readPlace(place: string) {
  try {
    const rows = await api.placeThreads(place);
    placeRows.value = { ...placeRows.value, [place]: rows };
  } catch (e) {
    error.value = String(e);
  }
}

// ---- read state (design §2.25): the cursor moves only when it really advances.
/** The open conversation's end is in view (the transcript, scrolled to the bottom). */
export const atEnd = signal(true);
/** The window has the user's attention. */
export const focused = signal(typeof document === "undefined" ? true : document.hasFocus());
if (typeof window !== "undefined" && window.addEventListener) {
  window.addEventListener("focus", () => (focused.value = true));
  window.addEventListener("blur", () => (focused.value = false));
}
/** What this client has sent as read, per thread: the newest `upto`. */
const readSent = new Map<string, string>();
let readPending: { id: string; upto: string } | null = null;
/** Reads are written at most every 30 s per burst while a turn streams (each
 * one is a record that syncs to every node); the first of a thread at once. */
const readSoon = debounced(() => void flushRead(), 3000, 30000);

/** Marks these conversations read (a section's "mark all read" too). */
export async function markRead(rows: { id: string; upto: string }[]) {
  const fresh = rows.filter((r) => { const s = readSent.get(r.id); return s === undefined || Date.parse(r.upto) > Date.parse(s); });
  if (!fresh.length) return;
  const done = new Map(fresh.map((r) => [r.id, r.upto]));
  for (const r of fresh) readSent.set(r.id, r.upto);
  threads.value = readThrough(threads.value, done);
  placeRows.value = Object.fromEntries(Object.entries(placeRows.value).map(([p, l]) => [p, readThrough(l, done)]));
  try {
    await Promise.all(fresh.map((r) => api.call({ Read: { thread: r.id, upto: r.upto } })));
  } catch (e) {
    for (const r of fresh) readSent.delete(r.id);
    error.value = String(e);
  }
}
export const markUnreadIn = (rows: Iterable<ThreadItem>) => markRead(unreadOf(rows));

/** Every conversation (of one place, when given) read, node-side, so it covers
 * rows the client has not loaded; the loaded ones show it at once. */
export async function markAllRead(place?: string) {
  const mine = (t: ThreadItem) => place === undefined || t.place === place;
  const done = new Map<string, string>();
  for (const t of [...threads.value, ...Object.values(placeRows.value).flat()]) if (mine(t) && t.unread && t.last_step) done.set(t.id, t.last_step);
  for (const [id, upto] of done) readSent.set(id, upto);
  threads.value = readThrough(threads.value, done);
  placeRows.value = Object.fromEntries(Object.entries(placeRows.value).map(([p, l]) => [p, readThrough(l, done)]));
  try {
    await api.call({ ReadAll: { place: place ?? null } });
    await refresh();
  } catch (e) {
    error.value = String(e);
    void refresh();
  }
}

async function flushRead() {
  const p = readPending;
  readPending = null;
  if (p) await markRead([p]);
}

/** After anything that can change what is shown: the open conversation, its
 * end in view, the window focused, and an `unread` row: read it. */
export function considerRead() {
  const row = current.value;
  if (!row || homeView.value || !api || !entries.value.length) return;
  const upto = readTarget(row, atEnd.value, focused.value, readSent.get(row.id));
  if (!upto) return;
  const first = !readSent.has(row.id);
  readPending = { id: row.id, upto };
  if (first) void flushRead();
  else readSoon();
}

/** Lists the threads again: at most one request in flight, one queued. */
export const refresh = coalesced(async () => {
  try {
    const list = await api.listThreads();
    threads.value = list.threads.sort((x, y) => +!!y.pinned - +!!x.pinned || (y.last_step ?? "").localeCompare(x.last_step ?? ""));
    placeCounts.value = list.places;
    // The places read on their own follow the list.
    await Promise.all(Object.keys(placeRows.value).map(readPlace));
    if (selected.value) await readExtra(selected.value);
  } catch (e) {
    error.value = String(e);
  }
});

/** The open thread's transcript again, soon: bursts (a tool result per
 * call, gaps) become one read. */
const reloadSoon = debounced(() => { if (selected.value) void reload(selected.value); }, 300, 1500);

/** Messages sent from a thread that is no longer open, which its transcript
 * did not have yet: shown again when it is opened (until then, a message
 * queued or woven into a turn was gone from it on coming back). */
const outboxes = new Map<string, O.UserEntry[]>();
/** How long a message kept so is shown unwritten. */
const OUTBOX_KEPT = 30 * 60_000;

function stash(id: string | null) {
  if (!id) return;
  const mine = O.locals(entries.value).filter((e) => e.outgoing !== "failed");
  if (mine.length) outboxes.set(id, mine);
  else outboxes.delete(id);
}

function unstash(id: string): O.UserEntry[] {
  const kept = outboxes.get(id) ?? [];
  outboxes.delete(id);
  return kept.filter((e) => Date.now() - Date.parse(e.at) < OUTBOX_KEPT);
}

/** Opens a thread. One compacted into another is the same conversation: it
 * opens as the newest (its transcript has the earlier part, above a divider),
 * unless `asWas` (the divider's link to it: read as it was, `latest` naming
 * where it goes on). */
export async function select(id: string, asWas = false, how: "push" | "replace" | "none" = "push"): Promise<number | undefined> {
  // The URL and its history entry first: whether the map is on screen decides the back button.
  Nav.opening(id, how);
  void flushRead();
  unfollow?.();
  stash(selected.value);
  latest.value = null;
  if (!asWas) {
    const row = threads.value.find((t) => t.earlier?.includes(id));
    if (row && row.id !== id) { id = row.id; if (how !== "none") Nav.opening(id, "replace"); }
  }
  selected.value = id;
  lastOpen.value = id;
  turn.value = null;
  turnStatus.value = null;
  speaking.value = null;
  { const r = threads.value.find((x) => x.id === id); room.value = r ? roomOfRow(r) : null; }
  homeView.value = false;
  const opened = nav.peek() + 1;
  nav.value = opened;
  entries.value = unstash(id);
  transcriptWindow.value = null;
  forkedFrom.value = null;
  if (!(id in extra.value)) extra.value = {};
  const [now] = await Promise.all([reload(id), readExtra(id)]);
  // Compacted since (a link, a notification, a list not refreshed yet).
  if (nav.peek() !== opened) return;
  if (now && now !== id) {
    if (!asWas) return select(now, false, how === "none" ? "none" : "replace");
    latest.value = now;
  }
  unfollow = api.follow(id, (f) => {
    if ("gap" in f) reloadSoon();
    else if ("turn" in f) { if (selected.value === id) turnStatus.value = f.turn; }
    else apply(id, f.event);
  });
  return opened;
}

/** Reads the transcript again (after a gap, or a turn's end). What arrived
 * from the turn's stream meanwhile is kept after it. */
let reading: { nav: number; again: boolean } | null = null;
/** The newest thread of the open one's compaction chain, as its transcript
 * said (`Transcript.current`). */
async function reload(id: string): Promise<string | undefined> {
  const route = nav.peek();
  if (reading?.nav === route) { reading.again = true; return; }
  const read = { nav: route, again: false };
  reading = read;
  let current: string | undefined;
  try {
    do {
      read.again = false;
      const keep = entries.value.length;
      // Opening: the conversation's end. Again: from the window's start on
      // (what "Show earlier" read stays); a rewind to before it, the end anew.
      const w = transcriptWindow.value;
      let t = await api.transcript(id, w ? { after: w.start } : { tail: WINDOW });
      let fresh = !w;
      if (w && w.start > 0 && !t.entries.length) {
        t = await api.transcript(id, { tail: WINDOW });
        fresh = true;
      }
      if (nav.peek() !== route || selected.value !== id) break;
      current = t.current;
      // While a turn runs, the stream's newer entries (past what was here when
      // reading began) go after the node's; at its end the node's are all.
      // Messages sent from here stay until the node's transcript has them.
      const tail = turn.value ? entries.value.slice(keep).filter((e) => !O.isLocal(e)) : [];
      // What "Show earlier" read meanwhile (before where this read began) stays.
      const head = fresh || !w ? [] : entries.value.filter((e) => "n" in e && e.n >= 0 && e.n < w.start && !O.isLocal(e));
      if (fresh) transcriptWindow.value = { start: t.start, earlier: t.earlier };
      entries.value = O.merge([...head, ...t.entries, ...tail], O.locals(entries.value));
      forkedFrom.value = t.forked_from;
      room.value = t.room ?? null;
    } while (read.again);
  } catch (e) {
    if (nav.peek() === route) error.value = String(e);
  } finally {
    if (reading === read) reading = null;
  }
  return current;
}

let earlierRead: { route: number; done: Promise<boolean> } | null = null;
/** Reads the window before the open transcript's ("Show earlier"): false
 * when there is none, or it could not. */
export function loadEarlier(): Promise<boolean> {
  const id = selected.peek();
  const w = transcriptWindow.peek();
  const route = nav.peek();
  if (!id || !w?.earlier) return Promise.resolve(false);
  if (earlierRead?.route === route) return earlierRead.done;
  loadingEarlier.value = true;
  const done = (async () => {
    try {
      const t = await api.transcript(id, { before: w.start, tail: WINDOW });
      if (nav.peek() !== route || transcriptWindow.peek() !== w || t.start >= w.start) return false;
      entries.value = [...t.entries, ...entries.peek()];
      transcriptWindow.value = { start: t.start, earlier: t.earlier };
      return true;
    } catch (e) {
      if (nav.peek() === route) error.value = String(e);
      return false;
    } finally {
      if (earlierRead?.route === route) {
        earlierRead = null;
        loadingEarlier.value = false;
      }
    }
  })();
  earlierRead = { route, done };
  return done;
}

/** Reads earlier windows until an entry matches (a digest item's or a
 * search hit's message), or there are none. */
export async function reveal(match: (e: Entry) => boolean, route = nav.peek()): Promise<boolean> {
  for (let i = 0; i < 100 && nav.peek() === route && !entries.peek().some(match); i++) {
    if (!(await loadEarlier())) break;
  }
  return nav.peek() === route && entries.peek().some(match);
}

export function apply(id: string, e: LiveEvent) {
  if (id !== selected.value) return;
  if (typeof e === "object" && "Compacted" in e) {
    // The conversation goes on in a new thread: the divider now, then the
    // new thread opened (its transcript has all of it, the divider too).
    const c = e.Compacted;
    entries.value = [...entries.value, { n: -1, kind: "compacted", at: new Date().toISOString(), said: compactedSaid(c), ...c }];
    if (latest.value !== null) latest.value = c.to;
    else if (c.to !== id) void select(c.to, false, "replace");
    return;
  }
  const list = entries.value.slice();
  if (e === "Started") {
    // A turn starts: its text streams into one entry until it ends; the
    // message it runs for is its own now.
    turn.value = { phase: "waiting", since: Date.now() };
    speaking.value = null;
    entries.value = O.stream(O.started(list), { kind: "live-assistant", text: "" });
    return;
  } else if (e === "Ended") {
    turn.value = null;
    speaking.value = null;
    entries.value = O.ended(list);
    void refresh();
    reloadSoon();
  } else if ("Thinking" in e) {
    if (turn.value?.phase !== "thinking") turn.value = { phase: "thinking", since: Date.now() };
    return;
  } else if ("Text" in e) {
    if (turn.value?.phase !== "writing") turn.value = { phase: "writing", since: Date.now() };
    // Above messages sent from here the turn has not taken yet (`liveAt`).
    entries.value = O.stream(list, { kind: "live-assistant", text: e.Text, ...(speaking.value ? { speaker: speaking.value } : {}) }, e.Text);
    return;
  } else if ("ToolCall" in e) {
    turn.value = { phase: "tool", since: Date.now() };
    entries.value = O.stream(list, { n: -1, kind: "tool", at: new Date().toISOString(), name: "", call: e.ToolCall, result: null });
    return;
  } else if ("ToolResult" in e) {
    turn.value = { phase: "waiting", since: Date.now() };
    const i = list.map((x) => x.kind === "tool" && !x.result).lastIndexOf(true);
    const call = list[i];
    if (call?.kind === "tool") list[i] = { ...call, result: e.ToolResult };
    // What the tool showed (images, a diff, an app) is in the node's
    // transcript now: read it, rather than wait for the turn's end.
    entries.value = list;
    reloadSoon();
    return;
  } else if ("Note" in e) {
    // A message woven into the turn comes back as its note: ours, read now.
    const [woven, ours] = O.wove(list, e.Note);
    const r = room.value;
    // Taken: it stays where it is, and what streams next goes after it
    // (a room takes it after the reply being written, design §2.27).
    if (ours) { entries.value = woven; return; }
    // A room's round (design §2.27): a speaker's `[Name · model]` opens its
    // bubble; a pass or a stop is a quiet note.
    const rn = r ? roomNote(e.Note, r.participants) : null;
    if (rn?.kind === "speaker") {
      speaking.value = rn.speaker;
      const fresh = { kind: "live-assistant" as const, text: "", speaker: rn.speaker };
      const at = O.liveAt(list);
      const prev = list[at - 1];
      if (prev?.kind === "live-assistant" && !prev.text.trim()) list[at - 1] = fresh;
      else list.splice(at, 0, fresh);
      entries.value = list;
      return;
    }
    // A question of the speaker's settled is no end of its turn.
    if (rn && rn.kind !== "room" && rn.kind !== "asked" && speaking.value?.slug === rn.slug) speaking.value = null;
    entries.value = O.stream(list, { kind: "note", text: e.Note });
    return;
  }
  else if ("Retry" in e) list.push({ kind: "note", text: "retrying: " + e.Retry });
  // `Thinking` is not in the transcript, so it is not shown.
  entries.value = list;
}

async function guarded(f: () => Promise<unknown>) {
  try {
    error.value = null;
    await f();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}

const NEW = "new";
const chatRequest = (thread: string | null, text: string, attachments: Staged[], scopes: StagedScope[] = []) => ({
  Chat: {
    thread, text, attachments: attachments.map(attachmentArg), model: picks.value[thread ?? NEW] || null, repo: null, cwd: null, global: true, env: [],
    tools: [], skills: [], place: thread ? null : newPlace.value, memory: null, end_user: null,
    node: thread ? null : newNode.value, spoken: false, scopes: scopeTargets(scopes),
  },
});

/** A message sent from here, shown until the node's transcript has it (its scopes as chips). */
export const optimistic = (text: string, files: Staged[], scopes: StagedScope[] = []): O.UserEntry => ({
  n: -1, kind: "user", from: null, at: new Date().toISOString(), text,
  images: files.filter((f) => f.type.startsWith("image/")),
  files: files.filter((f) => !f.type.startsWith("image/")),
  ...(scopes.length ? { scopes: chipsOf(scopes) } : {}),
  pending: true, outgoing: "sending",
});

/** How long a message's request may go unanswered before it is said failed. */
const SEND_TIMEOUT = 30_000;

/** Whether a turn runs on the open thread, as best known here. */
const runningNow = (t: ThreadItem) => !!turn.value || !!turnStatus.value?.running || t.state === "working";

/** A message held back because the node that owns its thread is offline: the
 * user picks another node to continue on (or this one), or drops it. */
export const offlineOwner = signal<{ thread: string; node: string; text: string; files: Staged[]; scopes: StagedScope[] } | null>(null);

/** Answer `offlineOwner`: continue on `node` (null: run here), or drop it. */
export const resolveOffline = async (node: string | null | "drop") => {
  const o = offlineOwner.value;
  if (!o) return;
  offlineOwner.value = null;
  if (node === "drop") return;
  await runOn(o.thread, node);
  await send(o.text, o.files, true, o.scopes);
};

export const send = (text: string, files: Staged[] = [], checked = false, scopes: StagedScope[] = []): Promise<void> =>
  guarded(async () => {
    const t = current.value;
    if (!t) return;
    // Typed into an earlier part of a compacted conversation: it goes to the
    // newest (the node sends it there too), which opens.
    if (selected.value !== t.id) await select(t.id);
    // Unpinned and owned by another node: that is where it continues, only if
    // the node is online. Offline, warn and propose another node.
    const here0 = attention.value?.node;
    if (!checked && here0 && t.node !== here0) {
      await loadRuns(t.id);
      const owner = runners.value.find((r) => r.name === t.node);
      if (pinned.value == null && owner && owner.runs == null) {
        offlineOwner.value = { thread: t.id, node: t.node, text, files, scopes };
        return;
      }
    }
    const mine = optimistic(text, files, scopes);
    entries.value = [...entries.value, mine];
    const set = (to: O.Outgoing, why?: string) => {
      if (selected.value === t.id) entries.value = O.mark(entries.value, mine.at, to, why);
      else if (outboxes.has(t.id)) outboxes.set(t.id, O.locals(O.mark(outboxes.get(t.id)!, mine.at, to, why)));
    };
    try {
      // A thread another node runs (its owner's by default, or the one the user
      // chose with `RunOn`) is typed as a `Chat`, the web client's: the node
      // passes it to that node, woven into a turn there or a turn of its own,
      // and the turn streams back on this request (nothing here follows it).
      // "Run here" (`RunOn` none) makes the thread this node's again.
      const here = attention.value?.node;
      if (here && t.node !== here) {
        set("starting");
        apply(t.id, "Started");
        try {
          await api.stream(chatRequest(t.id, text, files, scopes), (r) => {
            const ev = replyEvent(r);
            if (ev) apply(t.id, ev);
          });
        } finally {
          apply(t.id, "Ended");
        }
        return;
      }
      // Attachments go with a chat turn (`Send` has none): the files are
      // already in the node's store (`blake3:…`); its turn streams to followers.
      if (files.length || (picks.value[t.id] && !runningNow(t))) {
        set(runningNow(t) ? "queued" : "starting");
        await api.stream(chatRequest(t.id, text, files, scopes), () => {});
        return;
      }
      // Always `interject`: the node weaves it into the turn running, or
      // starts one (after the turn starting, if one is); what the list said
      // a moment ago is no ground to choose.
      const r = await api.call({ Send: { thread: t.id, text, from: null, keep: false, interject: true, scopes: scopeTargets(scopes) } }, SEND_TIMEOUT);
      const line = typeof r === "object" && "Lines" in r ? r.Lines[0] : undefined;
      const now = O.locals(entries.value).find((e) => e.at === mine.at);
      // The turn may have begun already (its `Started` came first).
      if (now?.outgoing === "sending") set(O.sentAs(line, runningNow(t)));
    } catch (e) {
      set("failed", e instanceof Error ? e.message : String(e));
      throw e;
    }
  });

/** New threads named by the node and not listed yet: the node writes a thread
 * with its first step, so until that step ends (a long first answer: a minute
 * of thinking) neither the list nor `Threads { thread }` has it. Their row
 * meanwhile, working, so the view shows the turn and its Cancel. */
const provisional = new Map<string, ThreadItem>();

const provisionalRow = (id: string, text: string): ThreadItem => {
  const now = new Date().toISOString();
  const line = text.split("\n").find((l) => l.trim())?.trim() ?? "Chat";
  return {
    id, title: line.length > 80 ? line.slice(0, 80) + "…" : line, goal: text,
    node: newNode.value ?? attention.value?.node ?? "", started_on: attention.value?.node ?? null, place: newPlace.value ?? "global",
    parent: null, parent_title: null, detached: false, read_only: false, app: null,
    state: "working", outcome: null, said: "working", question: null, started: now, last_step: now,
    model_calls: 0, cost: null, last_words: null, in_flight: null,
  };
};

/** A new thread: `Chat` streams on its own request; the thread is named by
 * its first reply, then followed like any other (its events come as events). */
export const newThread = (text: string, files: Staged[] = [], scopes: StagedScope[] = []) =>
  guarded(async () => {
    let opened: Promise<void> = Promise.resolve();
    let named: string | null = null;
    creating.value = { text, images: files, scopes };
    // The navigation the pending view belongs to: going home or elsewhere
    // meanwhile leaves it, and the named thread then does not pull the user back.
    const at = nav.value;
    const done = () => { if (creating.value?.text === text) creating.value = null; };
    try {
      await api.stream(chatRequest(null, text, files, scopes), (r) => {
        if (typeof r === "object" && "Thread" in r) {
          named = r.Thread;
          provisional.set(r.Thread, provisionalRow(r.Thread, text));
          // The stream lasts the whole first turn: the pending view ends here, once named.
          opened = refresh().then(async () => {
            if (nav.value !== at) { done(); return; }
            await select(r.Thread);
            done();
            // The node writes the message with the turn's first step.
            if (!entries.value.some((x) => x.kind === "user")) entries.value = [{ ...optimistic(text, files, scopes), outgoing: "running" }, ...entries.value];
            if (!turn.value && current.value?.state === "working") turn.value = { phase: "waiting", since: Date.now() };
          });
        }
      });
      await opened;
    } finally {
      done();
      // Its turn is over (or failed before writing anything): the node's row
      // from now on, or none.
      if (named) {
        provisional.delete(named);
        if (extra.value[named]?.state === "working" && selected.value === named) void readExtra(named);
      }
      newNode.value = null;
      if (picks.value[NEW]) {
        const m = { ...picks.value };
        delete m[NEW];
        picks.value = m;
      }
    }
    await refresh();
  });

export const answer = (question: string, allow: boolean, always = false, reason: string | null = null) =>
  guarded(async () => {
    await api.call({ Answer: { question, allow, reason, always } });
    await refresh();
  });

export const cancel = () =>
  guarded(async () => {
    const t = current.value;
    if (t) await api.call({ Cancel: { thread: t.id } });
    await refresh();
  });

/** A compaction's divider line, as the node says it (`Compaction::said`). */
export function compactedSaid(c: Compaction): string {
  const figures = c.tokens != null
    ? `${Math.floor(c.tokens / 1000)}k of ${Math.floor(c.window / 1000)}k window`
    : `past the ${Math.floor(c.window / 1000)}k window`;
  const rest = c.kept === 0 ? "a summary" : c.kept === 1 ? "summary + the last message" : `summary + last ${c.kept} messages`;
  return `compacted here (${figures}): ${rest}`;
}

/** Shows what a route says (the browser's back and forward, a reload, a link). */
export function applyRoute(r: Route) {
  if (r.kind === "thread") {
    if (selected.value !== r.id || homeView.value) void select(r.id, false, "none");
  } else if (!homeView.value || selected.value) goHome("none");
}
