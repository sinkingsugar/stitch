// Where a thread runs (design §2.7, "Continue on another node"): the user's
// `runs-on` choice (`RunsOn`), the other nodes that run programs (`Runners`)
// and continuing there (`RunOn`) -- the requests the phone's thread menu uses.
import { signal } from "@preact/signals";
import type { Api } from "../conn";
import type { Reply } from "../api/Reply";
import type { LiveEvent } from "../model";

let api: Api;
export const pinned = signal<string | null>(null);
/** A node `Runners` lists. */
export type Runner = { name: string; runs: boolean | null; otherBuild?: boolean };
export const runners = signal<Runner[]>([]);

/** What a node's name is marked with in a menu: " (offline)", or " (other
 * build)" for one reached on a build this node cannot read. */
export function unreached(r: Runner): string {
  return r.otherBuild ? " (other build)" : r.runs == null ? " (offline)" : "";
}
export const runsError = signal<string | null>(null);

export function initRuns(a: Api) { api = a; }

/** The streamed replies of a `Chat` the node passed to another node, as the
 * events a followed turn gives. */
export function replyEvent(r: Reply): LiveEvent | null {
  if (typeof r !== "object") return null;
  if ("Text" in r) return { Text: r.Text };
  if ("Thinking" in r) return { Thinking: r.Thinking };
  if ("ToolCall" in r) return { ToolCall: r.ToolCall };
  if ("ToolResult" in r) return { ToolResult: r.ToolResult };
  if ("Note" in r) return { Note: r.Note };
  if ("Retry" in r) return { Retry: r.Retry };
  if ("Compacted" in r) return { Compacted: r.Compacted };
  return null;
}

export async function loadRuns(thread: string) {
  try {
    pinned.value = null;
    const [on, rs] = await Promise.all([api.runsOn(thread), api.runners()]);
    pinned.value = on;
    runners.value = rs;
  } catch (e) {
    runsError.value = String(e instanceof Error ? e.message : e);
  }
}

/** The nodes a new thread can be put on (`Chat.node`, design §2.7, "A new
 * chat on another node"): those that run programs, or were not reached. */
export async function loadRunners() {
  try {
    runners.value = await api.runners();
  } catch (e) {
    runsError.value = String(e instanceof Error ? e.message : e);
  }
}

/** Continue on `node`; null brings the thread back here ("Run here"). */
export async function runOn(thread: string, node: string | null) {
  try {
    runsError.value = null;
    await api.runOn(thread, node);
  } catch (e) {
    runsError.value = String(e instanceof Error ? e.message : e);
  }
  await loadRuns(thread);
}
