// Thread actions (design §2.24, batch 3b): the requests behind the header
// menu, the per-message actions and the search box. Each uses an existing
// `Request`: Rename, Pin, Fork (also compact and summarize), ReadOnly, Models
// and EffectiveModel, Toolset, DeleteThreads, Rewind, Search.
import { signal } from "@preact/signals";
import * as S from "../store";
import type { SearchHit } from "../api/SearchHit";
import type { SetItem } from "../api/SetItem";
import type { Entry } from "../model";
import type { StagedScope } from "../scopes";
import { parse as parseDoomed, type Doomed } from "./doomed";

export type Dialog =
  | { k: "rename" }
  | { k: "fork"; upto: string | null; preset: "fork" | "compact" | "summarize" }
  | { k: "delete" }
  | { k: "rewind"; id: string; text: string }
  | { k: "tools" };

/** Text for the composer to take in place of what it holds (edit & resend). */
export const edit = signal<{ text: string; files: import("../transcript/blob").Staged[]; scopes?: StagedScope[] } | null>(null);
export const dialog = signal<Dialog | null>(null);
export const menuOpen = signal(false);
/** Text for the composer to take (a quote); the composer clears it. */
export const draft = signal<string | null>(null);
/** A message to scroll to and mark (a search hit). */
export const focusMsg = signal<{ mid: string; route: number } | null>(null);
export const focusedMid = () => {
  const focus = focusMsg.value;
  return focus && focus.route === S.nav.value ? focus.mid : null;
};
export const busy = signal<string | null>(null);

/** The "new thread" form is shown (the sidebar's ＋, "Ask about this"); any navigation closes it (app.tsx). */
export const showNew = signal(false);
/** Scopes for a composer to stage (design §2.27): the map's "Ask about this",
 * its tray, the thread menu. `to`: the thread whose composer takes them, null
 * for the new thread's. The composer clears it. */
export const pendingScopes = signal<{ to: string | null; scopes: StagedScope[] } | null>(null);

/** A new thread with these scopes staged, in `place` (a thread row's place label) when given. */
export async function askInNew(scopes: StagedScope[], place?: string | null) {
  if (!scopes.length) return;
  pendingScopes.value = { to: null, scopes };
  showNew.value = true;
  if (place == null) return;
  await S.loadPlaces();
  const p = S.places.value.find((x) => x.label === place || x.key === place);
  if (p) S.newPlace.value = p.key;
  else if (place === "global") S.newPlace.value = null;
}

/** Stages these scopes in an existing conversation's composer, opening it. */
export async function attachTo(thread: string, scopes: StagedScope[]) {
  if (!scopes.length) return;
  pendingScopes.value = { to: thread, scopes };
  if (S.selected.value !== thread || S.homeView.value) await S.select(thread);
}

const fail = (e: unknown) => {
  S.error.value = e instanceof Error ? e.message : String(e);
};

/** Runs an action: `busy` says what is happening, an error shows in the thread. */
export async function run<T>(what: string, f: () => Promise<T>): Promise<T | undefined> {
  busy.value = what;
  S.error.value = null;
  try {
    return await f();
  } catch (e) {
    fail(e);
  } finally {
    busy.value = null;
  }
}

export const rename = (thread: string, title: string) =>
  run("renaming", async () => {
    await S.rpc({ Rename: { thread, title } });
    await S.refresh();
  });

export const pin = (thread: string, on: boolean) =>
  run("pinning", async () => {
    await S.rpc({ Pin: { thread, on } });
    await S.refresh();
  });

export const readOnly = (thread: string, on: boolean) =>
  run("switching mode", async () => {
    await S.rpc({ ReadOnly: { thread, on } });
    await S.refresh();
  });

export interface ForkOpts {
  upto: string | null;
  last: number | null;
  no_attachments: boolean;
  no_tools: boolean;
  summarize: boolean;
}

/** Forks (compact and summarize are forks with a summary) and opens the new thread. */
export const fork = (thread: string, o: ForkOpts) =>
  run(o.summarize ? "summarizing (a model call)" : "forking", async () => {
    const r = await S.rpc({ Fork: { thread, ...o } });
    if (typeof r === "object" && "Forked" in r) {
      await S.refresh();
      await S.select(r.Forked);
      return true;
    }
    throw new Error("unexpected reply to fork");
  });

export const rewind = (thread: string, upto: string | null) =>
  run("rewinding", async () => {
    await S.rpc({ Rewind: { thread, upto } });
    await S.refresh();
    await S.select(thread);
    return true;
  });

/** What deleting a thread takes with it (its compaction chain), each with
 *  its role (`doomed.ts`). */
export async function doomed(thread: string): Promise<Doomed[]> {
  const r = await S.rpc({ DeleteThreads: { threads: [thread], spawned_by: null, dry_run: true, from_thread: null } });
  if (typeof r !== "object" || !("Lines" in r)) throw new Error("unexpected reply");
  return r.Lines.map(parseDoomed);
}

export const remove = (thread: string) =>
  run("deleting", async () => {
    await S.rpc({ DeleteThreads: { threads: [thread], spawned_by: null, dry_run: false, from_thread: null } });
    S.goHome();
    await S.refresh();
  });

export interface ModelList {
  models: string[];
  problems: string[];
  default: string;
}
export async function models(): Promise<ModelList> {
  const r = await S.rpc({ Models: { node: null } });
  if (typeof r !== "object" || !("Models" in r)) throw new Error("unexpected reply");
  return r.Models;
}
/** The model the next turn would use (the node's resolver), given the pick. */
export async function effectiveModel(thread: string | null, pick: string | null, place: string | null = null): Promise<string> {
  const r = await S.rpc({ EffectiveModel: { thread, place, cwd: null, model: pick } });
  if (typeof r !== "object" || !("Lines" in r)) throw new Error("unexpected reply");
  return r.Lines[0] ?? "";
}
/** `thread`: a thread's ID, or "new" for the next new thread. */
export const pickModel = (thread: string, model: string | null) => {
  const m = { ...S.picks.value };
  if (model) m[thread] = model;
  else delete m[thread];
  S.picks.value = m;
};

export interface Sets {
  tools: SetItem[];
  skills: SetItem[];
}
/** A thread's tool and skill sets; `+name` / `-name` entries are written first. */
export async function toolset(thread: string, tools: string[] = [], skills: string[] = []): Promise<Sets> {
  const r = await S.rpc({ Toolset: { thread, cwd: null, global: false, tools, skills } });
  if (typeof r !== "object" || !("Toolset" in r)) throw new Error("unexpected reply");
  return r.Toolset;
}

export async function search(query: string): Promise<SearchHit[]> {
  const r = await S.rpc({ Search: { query, here: false, cwd: null, limit: 40 } });
  if (typeof r !== "object" || !("Hits" in r)) throw new Error("unexpected reply");
  return r.Hits;
}

/** Opens a hit's thread and scrolls to its message when it is shown. */
export async function openHit(h: SearchHit) {
  focusMsg.value = null;
  const route = await S.select(h.thread);
  if (route === undefined || route !== S.nav.peek()) return;
  const mid = `${h.thread}#${h.msg}`;
  // Earlier than the transcript's window: read back to the exact source.
  if (!(await S.reveal((e) => "mid" in e && e.mid === mid, route))) {
    if (route === S.nav.peek()) S.error.value = "This search result’s message could not be found in the transcript.";
    return;
  }
  if (route === S.nav.peek()) focusMsg.value = { mid, route };
}

export function quote(text: string) {
  draft.value = text.split("\n").map((l) => "> " + l).join("\n") + "\n\n";
}

export async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    S.error.value = null;
  } catch (e) {
    fail(e);
  }
}

/** The transcript as Markdown. */
export function markdown(title: string, items: Entry[]): string {
  const out = [`# ${title}`, ""];
  for (const e of items) {
    if (e.kind === "user") out.push("## You", "", e.text, "");
    else if (e.kind === "assistant") out.push("## Assistant", "", e.text, "");
    else if (e.kind === "tool") {
      out.push(`- \`${e.call.replace(/`/g, "'")}\`${e.result && !e.result.ok ? " (failed)" : ""}`);
    } else if (e.kind === "failed") out.push(`> ${e.text}`, "");
  }
  return out.join("\n").replace(/\n{3,}/g, "\n\n") + "\n";
}
