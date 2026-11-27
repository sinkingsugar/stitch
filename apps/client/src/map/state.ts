// The map's state: the forest (read when the map is first shown, again on the
// node's change signal while it is shown), the model built from it, the
// filters, the selection. Signals, so they outlive the view that shows them.
import { computed, effect, signal } from "@preact/signals";
import type { ThreadItem } from "../model";
import { coalesced } from "../coalesce";
import * as S from "../store";
import { merge } from "../deltas";
import { mapShown } from "../nav";
import { scopeOfSel, toggleScope, type StagedScope } from "../scopes";
import { applyFilter, buildModel, defaultFilter, emptyModel, gone, sameSel, selOfThread, type MFilter, type Model, type Sel, type Stats } from "./model";

export const model = signal<Model>(emptyModel());
export const stats = signal<Stats>(applyFilter(emptyModel(), defaultFilter(), Date.now()));
export const filter = signal<MFilter>(defaultFilter());
export const sel = signal<Sel | null>(null);
export const hover = signal<Sel | null>(null);
/** What the engine must do about the last change: re-pack (`data`) or restyle (`filter`). */
export const change = signal<{ n: number; kind: "data" | "filter" }>({ n: 0, kind: "data" });
export const loaded = signal(false);
export const loading = signal(false);
export const failure = signal<string | null>(null);
/** The map has been shown once: its view is mounted from then on (hidden while a conversation is open). */
export const mounted = signal(false);
/** The map's scope tray (design §2.27): efforts and threads picked with
 * Shift+click or "+ Scope", to ask about in a new chat or attach to one. */
export const picked = signal<StagedScope[]>([]);
/** The IDs of what is picked, for the engines' rings. */
export const pickedIds = computed<ReadonlySet<string>>(() => new Set(picked.value.map((s) => s.id)));
/** Picks what `s` stands for (an effort with spawned threads, else a thread), or unpicks it. */
export function togglePick(s: Sel | null) {
  const sc = scopeOfSel(model.peek(), s);
  if (sc) picked.value = toggleScope(picked.value, sc);
}
/** Whether what `s` stands for is picked. */
export const isPicked = (s: Sel | null) => {
  const sc = scopeOfSel(model.value, s);
  return !!sc && picked.value.some((x) => x.kind === sc.kind && x.id === sc.id);
};

/** Where the last selection came from (`islands`: the canvas; the engine flies only for the others). */
export const selFrom = { v: "ui" };

let rows: ThreadItem[] = [];
/** Deltas applied to `rows` while the map was hidden, not laid out yet. */
let unshown = false;
let sig = "";
let layoutSig = "";
let stale = false;
let n = 0;

/** What changing would show on the map: a row's id, state, last step and so on. */
export const rowsSignature = (l: ThreadItem[]): string =>
  l.map((r) => `${r.id}|${r.state}|${r.last_step ?? ""}|${r.unread ? 1 : 0}|${r.question?.id ?? ""}|${r.title}|${r.cost ?? 0}|${r.place}|${r.parent ?? ""}|${r.digest?.at ?? ""}`).join("\n");

function bump(kind: "data" | "filter") {
  change.value = { n: ++n, kind };
}

/** Builds the model of `list` (when it differs from the last), applies the filters. */
export function setRows(list: ThreadItem[]) {
  const s = rowsSignature(list);
  if (loaded.value && s === sig) return;
  sig = s; rows = list;
  const now = Date.now();
  model.value = buildModel(rows, now);
  loaded.value = true;
  refilter(true, now);
}

/** Applies the filters to the model: restyle, or re-pack when places or efforts appear or vanish. */
export function refilter(newData: boolean, now = Date.now()) {
  const m = model.value;
  const st = applyFilter(m, filter.value, now);
  const layoutChanged = st.layoutSig !== layoutSig;
  layoutSig = st.layoutSig;
  stats.value = st;
  if (gone(m, sel.value)) { selFrom.v = "filter"; sel.value = null; }
  if (hover.value && gone(m, hover.value)) hover.value = null;
  bump(newData || layoutChanged ? "data" : "filter");
}

export function setFilter(patch: Partial<MFilter>) {
  filter.value = { ...filter.value, ...patch };
  refilter(false);
}

export function select(s: Sel | null, from: string) {
  if (sameSel(sel.value, s)) { if (s) { selFrom.v = from; sel.value = s ? { ...s } : s; } return; }
  selFrom.v = from;
  sel.value = s;
}

/** Reads the whole forest (`Threads { forest }`, one request, ~0.5 s for 1600 rows). */
export const load = coalesced(async () => {
  loading.value = true;
  try {
    setRows(await S.loadForest());
    failure.value = null;
    stale = false;
  } catch (e) {
    failure.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
});

/** The map is on screen: read the forest when it was never read or went stale. */
export function shown() {
  mounted.value = true;
  fresh();
}

/** The model is about to be read off the map (the composer's ⎇ picker): bring it up to date, as `shown` does — read
 * the forest when it was never read or went stale, else lay out the deltas that came while the map was hidden. */
export function fresh() {
  if (!loaded.value || stale) void load();
  else if (unshown) {
    unshown = false;
    setRows(rows);
  }
}

// The node's change signal (the same one the list follows, already debounced by the store): read again when the
// map is on screen, else only remember that it went stale; `setRows` drops a reading that changed nothing.
effect(() => {
  void S.changeTick.value;
  if (!loaded.value) return;
  if (mapShown.value) void load();
  else stale = true;
});

// The list's deltas (design §2.24): the rows that moved, put in the forest as they are (no read of it).
effect(() => {
  const d = S.listDelta.value;
  if (!d || !loaded.value) return;
  const next = merge(rows, d.upsert, d.remove, Number.MAX_SAFE_INTEGER);
  if (mapShown.peek()) setRows(next);
  else {
    rows = next; // laid out when shown
    unshown = true;
  }
});

// Two-way sync: a conversation opened from anywhere (the sidebar, a search hit, a notification, a link) is the map's
// selection too, and the map eases to it when it is shown. What the map itself opened is already selected.
effect(() => {
  const id = S.selected.value;
  if (!id || !loaded.value) return;
  const m = model.peek();
  const row = S.current.peek();
  const t = m.byId.get(row?.id ?? id) ?? m.byId.get(id);
  if (!t || t.synthetic) return;
  const want = selOfThread(t);
  const cur = sel.peek();
  if (cur && cur.id === want.id) return;
  if (t.effort && !t.effort.vis) setFilter({ time: "all" });
  select(want, "open");
});

/** What the card needs to show the conversation open in the client: its id, as listed. */
export const openedId = () => S.current.value?.id ?? S.selected.value;

/** Opens what is selected (a thread, or an effort's root) in the client's transcript. */
export function openSel(s: Sel | null) {
  const m = model.peek();
  const t = s && s.kind !== "place" ? (s.kind === "effort" ? m.eById.get(s.id)?.root : m.byId.get(s.id)) : null;
  if (t && !t.synthetic) void S.select(t.id);
}
