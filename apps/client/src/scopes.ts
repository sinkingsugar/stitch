// Thread and effort scopes (design §2.27, rooms.md §3, §5): what a message
// carries as `scopes` (`ScopeTarget`), staged in the composer as chips, picked
// on the map, shown on a sent message. Pure: no signals, no DOM, so the
// composer, the map and the tests share it (test/scopes.test.ts).
import type { ScopeTarget } from "./api/ScopeTarget";
import type { ScopeChip, ThreadRow } from "./payload";
import type { Model, MThread, Sel } from "./map/model";

/** A scope staged for a message: the target, with what the chip shows. */
export interface StagedScope {
  kind: "effort" | "thread";
  id: string;
  title?: string | null;
  /** An effort's threads (the root and everything spawned under it). */
  threads?: number;
}

export const scopeKey = (s: { kind: string; id: string }) => `${s.kind}:${s.id}`;

/** What goes on the wire: kind and id alone. */
export const scopeTargets = (l: StagedScope[]): ScopeTarget[] => l.map(({ kind, id }) => ({ kind, id }));

/** The chips a message sent from here shows until the node's transcript has it. */
export const chipsOf = (l: StagedScope[]): ScopeChip[] =>
  l.map((s) => ({ kind: s.kind, id: s.id, title: s.title ?? null, threads: s.threads ?? (s.kind === "effort" ? 0 : 1) }));

/** Adds scopes, each target once. One ID is one target: attaching a thread's
 * effort replaces the thread alone, and the other way round (the latest wins). */
export function addScopes(list: StagedScope[], add: StagedScope[]): StagedScope[] {
  let out = list.slice();
  for (const s of add) {
    const i = out.findIndex((x) => x.id === s.id);
    const was = out[i];
    if (was) {
      const next: StagedScope = { kind: s.kind, id: s.id, title: s.title ?? was.title };
      const n = s.kind === "effort" ? s.threads ?? (was.kind === "effort" ? was.threads : undefined) : undefined;
      if (n != null) next.threads = n;
      out[i] = next;
    }
    else out = [...out, s];
  }
  return out;
}

/** Removes the target when it is there (the same kind and ID), else adds it. */
export function toggleScope(list: StagedScope[], s: StagedScope): StagedScope[] {
  return list.some((x) => x.kind === s.kind && x.id === s.id) ? list.filter((x) => !(x.kind === s.kind && x.id === s.id)) : addScopes(list, [s]);
}

/** The chip's words: `Effort: <title> · N threads`, or the thread's title. */
export function scopeLabel(s: StagedScope): string {
  const title = s.title || s.id;
  if (s.kind === "thread") return title;
  return `Effort: ${title}${s.threads ? ` · ${s.threads} thread${s.threads === 1 ? "" : "s"}` : ""}`;
}

/** How many threads the scopes cover, as far as known (a thread is one). */
export const scopeThreads = (l: StagedScope[]) => l.reduce((n, s) => n + (s.kind === "effort" ? s.threads ?? 1 : 1), 0);

/** The message sent when only scopes are attached (no text typed). */
export function scopePrompt(l: StagedScope[]): string {
  const one = l.length === 1 ? (l[0]!.kind === "effort" ? "this effort" : "this conversation") : "these attached threads";
  return `Brief me on ${one}: where it stands, what was decided, and what is still open.`;
}

const THREAD_ID = /^\d{4}-\d{2}-\d{2}T\d{4,6}Z-[A-Za-z0-9_.-]+$/;

/** A pasted target: `effort:<id>`, `thread:<id>`, or a bare thread ID (an
 * anchor's `#m…` dropped). Null when it is none. */
export function parseScope(text: string): StagedScope | null {
  let v = text.trim();
  let kind: "effort" | "thread" = "thread";
  const m = /^(effort|thread):(.*)$/.exec(v);
  if (m) { kind = m[1] as "effort" | "thread"; v = m[2]!.trim(); }
  v = v.replace(/#.*$/, "");
  return THREAD_ID.test(v) ? { kind, id: v } : null;
}

/** What a thread of the map stands for as a scope: its effort when it is the
 * root of one with spawned threads, else the conversation alone. None for the
 * synthetic "(parent not loaded)" threads. */
export function scopeOfThread(t: MThread): StagedScope | null {
  if (t.synthetic) return null;
  const e = t.effort;
  if (e && e.root === t && e.threads.length > 1) return { kind: "effort", id: t.id, title: t.title || t.id, threads: e.threads.length };
  return { kind: "thread", id: t.id, title: t.title || t.id };
}

/** A map selection as a scope (a place: none). */
export function scopeOfSel(m: Model, s: Sel | null): StagedScope | null {
  if (!s || s.kind === "place") return null;
  if (s.kind === "effort") {
    const e = m.eById.get(s.id);
    return e ? scopeOfThread(e.root) : null;
  }
  const t = m.byId.get(s.id);
  return t && !t.synthetic ? { kind: "thread", id: t.id, title: t.title || t.id } : null;
}

/** The effort a thread belongs to, as a scope, when it has more than this thread. */
export function effortOfThread(t: MThread): StagedScope | null {
  const e = t.effort;
  if (!e || e.threads.length < 2) return null;
  const root = e.root.synthetic ? t : e.root;
  return { kind: "effort", id: root.id, title: root.title || root.id, threads: e.threads.length };
}

/** The scope "Ask about this" attaches for a thread: its effort when it has
 * spawned threads (the whole tree it is in, from its root), else the
 * conversation alone. From the map's model
 * when it has the thread, else from the list (`rows`: direct children). */
export function askScope(id: string, title: string | null, m: Model | null, rows: ThreadRow[]): StagedScope {
  const t = m?.byId.get(id);
  if (t && !t.synthetic) return (t.kids.length ? effortOfThread(t) : null) ?? { kind: "thread", id, title: t.title || title || id };
  // The list alone: the effort's size is not known here (the node resolves it).
  return rows.some((r) => r.parent === id) ? { kind: "effort", id, title: title ?? id } : { kind: "thread", id, title: title ?? id };
}

/** One row of the composer's picker: a thread, and its effort when it has one. */
export interface Candidate {
  id: string;
  title: string;
  place: string;
  /** Last step, ms. */
  e: number;
  thread: StagedScope;
  effort: StagedScope | null;
}

/** The picker's rows for `q` (every word in the title, place or ID), the
 * latest first: from the map's whole forest when it is loaded, else the list. */
export function candidates(q: string, rows: ThreadRow[], m: Model | null, limit = 12): Candidate[] {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const hit = (hay: string) => words.every((w) => hay.includes(w));
  const out: Candidate[] = [];
  if (m && m.threads.length) {
    for (const t of m.threads) {
      if (t.synthetic || !hit(`${t.title}\n${t.placeKey}\n${t.id}`.toLowerCase())) continue;
      out.push({ id: t.id, title: t.title || t.id, place: t.placeKey, e: t.e, thread: { kind: "thread", id: t.id, title: t.title || t.id }, effort: effortOfThread(t) });
    }
  } else {
    for (const r of rows) {
      if (!hit(`${r.title}\n${r.place}\n${r.id}`.toLowerCase())) continue;
      const kids = rows.some((x) => x.parent === r.id);
      out.push({
        id: r.id, title: r.title || r.id, place: r.place, e: Date.parse(r.last_step ?? "") || 0,
        thread: { kind: "thread", id: r.id, title: r.title || r.id },
        effort: kids ? { kind: "effort", id: r.id, title: r.title || r.id } : null,
      });
    }
  }
  return out.sort((a, b) => b.e - a.e).slice(0, limit);
}
