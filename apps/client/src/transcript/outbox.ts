// Messages sent from here that the node's transcript does not have yet, and
// where each stands (design §2.24, "sending feedback"). Pure functions over
// the transcript's entries, so they are tested without a node
// (outbox.test.ts).
import type { Entry } from "../model";

/** Where a message sent from here stands, until the transcript has it:
 * - `sending`: the request is out, no answer yet;
 * - `starting`: the node took it and starts a turn for it (its preamble runs);
 * - `queued`: the node took it, and its turn waits for the one running;
 * - `weaving`: woven into the running turn, which reads it at its next step;
 * - `running`: the turn has it (it is written with the turn's first step);
 * - `failed`: not sent (`failed` says why). */
export type Outgoing = "sending" | "starting" | "queued" | "weaving" | "running" | "failed";

/** A message entry (the user's or the model's: one shape, told apart by `kind`). */
export type UserEntry = Extract<Entry, { kind: "user" | "assistant" }>;

export const isLocal = (e: Entry): e is UserEntry => e.kind === "user" && !!e.pending;

/** What the node's answer to `Send` says became of the message
 * (`weave::Sent::said`, in `Reply::Lines`). */
export function sentAs(line: string | undefined, running: boolean): Outgoing {
  if (line && /woven into its running turn/.test(line)) return "weaving";
  if (line && /passed on to that node/.test(line)) return "starting";
  // A room (`rooms::send`, design §2.27): the round running takes it before
  // its next speaker, or a round starts on it.
  if (line && /taken by the round running/.test(line)) return "weaving";
  if (line && /a round runs on it/.test(line)) return "starting";
  return running ? "queued" : "starting";
}

/** The node's transcript, then the messages sent from here it does not have
 * yet (a user message of the same text written since it was sent is it). */
export function merge(node: Entry[], local: UserEntry[]): Entry[] {
  // What of each user entry no message of ours stood for yet: the node
  // writes the messages that came in a row from one sender as one (two
  // woven into the same step: their texts a line apart), so each of
  // ours takes its part of it.
  const left = new Map<number, string>();
  const kept = local.filter((l) => {
    if (l.outgoing === "failed") return true;
    const since = Date.parse(l.at) - 60_000;
    const mine = l.text.trim();
    for (let j = 0; j < node.length; j++) {
      const e = node[j]!;
      if (e.kind !== "user" || Date.parse(e.at) < since) continue;
      const rest = left.get(j) ?? e.text.trim();
      const cut = part(rest, mine);
      if (cut === null) continue;
      left.set(j, cut);
      return false;
    }
    return true;
  });
  return [...node, ...kept];
}

/** `text` without `mine` when `mine` is it or one of its lines' runs
 * (null when it is not in it). */
function part(text: string, mine: string): string | null {
  if (text === mine) return "\u0000";
  if (!mine || text === "\u0000") return null;
  const lines = text.split("\n");
  const want = mine.split("\n");
  for (let i = 0; i + want.length <= lines.length; i++) {
    if (want.every((w, k) => lines[i + k]!.trim() === w.trim()))
      return [...lines.slice(0, i), ...lines.slice(i + want.length)].join("\n");
  }
  return null;
}

/** Where what the running turn streams goes: before the messages sent from
 * here that it has not taken yet (they stay last, under it), after those
 * it took. */
export function liveAt(list: Entry[]): number {
  let i = list.length;
  while (i > 0) {
    const e = list[i - 1]!;
    if (!isLocal(e) || e.outgoing === "running") break;
    i--;
  }
  return i;
}

/** Puts a streamed entry in (`liveAt`), or, for text, adds to the one
 * streaming there. */
export function stream(list: Entry[], add: Entry, text?: string): Entry[] {
  const out = list.slice();
  const i = liveAt(out);
  const prev = out[i - 1];
  if (text !== undefined && prev?.kind === "live-assistant") out[i - 1] = { ...prev, text: prev.text + text };
  else out.splice(i, 0, add);
  return out;
}

/** The messages sent from here, in the order sent. */
export const locals = (list: Entry[]): UserEntry[] => list.filter(isLocal);

/** Sets the state of local message `at` (its `at` stamps it). */
export function mark(list: Entry[], at: string, to: Outgoing, failed?: string): Entry[] {
  return list.map((e) => (isLocal(e) && e.at === at ? { ...e, outgoing: to, failed } : e));
}

/** A turn began: the oldest message waiting for one is its message. */
export function started(list: Entry[]): Entry[] {
  const i = list.findIndex((e) => isLocal(e) && (e.outgoing === "starting" || e.outgoing === "queued"));
  if (i < 0) return list;
  const out = list.slice();
  out[i] = { ...(out[i] as UserEntry), outgoing: "running" };
  return out;
}

/** The running turn read a message woven into it (its `Note` is the text):
 * the message is the turn's now. Returns the list, and whether it matched
 * (the note is then not shown again). */
export function wove(list: Entry[], note: string): [Entry[], boolean] {
  // Still `sending`: the turn read it before the node's answer came.
  const i = list.findIndex((e) => isLocal(e) && (e.outgoing === "weaving" || e.outgoing === "sending") && e.text.trim() === note.trim());
  if (i < 0) return [list, false];
  const out = list.slice();
  out[i] = { ...(out[i] as UserEntry), outgoing: "running" };
  return [out, true];
}

/** The turn ended: what it had not read yet starts a turn of its own on the
 * node (`Steering`'s drop); what it ran is in the transcript once read. */
export function ended(list: Entry[]): Entry[] {
  return list.map((e) => (isLocal(e) && e.outgoing === "weaving" ? { ...e, outgoing: "starting" } : e));
}

/** A line for the message's state, under it. */
export function said(e: UserEntry, room = false): string | null {
  switch (e.outgoing) {
    case "sending": return "sending…";
    case "starting": return room ? "sent · the room starts a round…" : "sent · starting a turn…";
    case "queued": return "queued · runs when the current turn ends";
    case "weaving": return room ? "waiting · the room takes it after the current speaker" : "queued · the running turn reads it at its next step";
    case "failed": return "not sent: " + (e.failed || "unknown error");
    default: return null;
  }
}

/** Whether something of ours waits for a turn to show (the working line). */
export const awaiting = (list: Entry[]) =>
  list.some((e) => isLocal(e) && (e.outgoing === "starting" || e.outgoing === "queued" || e.outgoing === "sending"));
