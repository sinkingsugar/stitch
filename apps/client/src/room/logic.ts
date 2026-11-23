// Rooms in the web client (design §2.27, rooms.md §4, §5): what the views
// need, pure (no signals, no DOM), so it is tested (test/rooms.test.ts).
// Speaker colours, the `@slug` mentions of the composer, the room's notes
// as the node streams them (`rooms.rs`), and the requests `RoomCreate` and
// `RoomUpdate`.
import type { Request } from "../api/Request";
import type { RoomParticipantSpec } from "../api/RoomParticipantSpec";
import type { RoomInfo, RoomParticipant, Speaker, ThreadRow } from "../payload";
import { scopeTargets, type StagedScope } from "../scopes";

// ---- speakers

/** A slug's hue, the same everywhere (FNV-1a over its characters). */
export function speakerHue(slug: string): number {
  let h = 0x811c9dc5;
  for (const c of slug.toLowerCase()) {
    h ^= c.codePointAt(0)!;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h % 360;
}

/** A participant's accent colour (its dot, its header). */
export const speakerColor = (slug: string) => `hsl(${speakerHue(slug)} 70% 66%)`;

/** A model without its provider: `anthropic/claude-opus-5-5` → `claude-opus-5-5`. */
export const shortModel = (m: string | null | undefined) => (m ? m.slice(m.indexOf("/") + 1) : "");

/** A model's provider: `anthropic/claude-opus-5-5` → `anthropic` (none: ""). */
export const providerOf = (m: string | null | undefined) => (m && m.includes("/") ? m.slice(0, m.indexOf("/")) : "");

/** A participant's own row from the list (its cost, node, state, mode). */
export const rowOf = (p: { thread: string }, rows: ThreadRow[]) =>
  rows.find((r) => r.id === p.thread || !!r.earlier?.includes(p.thread)) ?? null;

/** The room a thread belongs to as a participant: the room it was spawned
 * under, else the room of its latest relayed message. */
export function roomOfThread(t: ThreadRow, rows: ThreadRow[], relayedFrom: string | null = null): string | null {
  if (t.parent && rows.some((r) => r.id === t.parent && r.room)) return t.parent;
  return relayedFrom;
}

// ---- the room's notes (`rooms.rs`: `Out::note`)

export type RoomNote =
  /** `[<Name> · <model>]`: a participant speaks now (or a council's reply follows). */
  | { kind: "speaker"; speaker: Speaker }
  /** `[<Name> passed]`, `[<Name> was stopped]`, `[<Name> said nothing]`, `[<Name> did not answer: …]`. */
  | { kind: "pass" | "stopped" | "silent" | "failed"; name: string; slug: string | null; text: string }
  /** The room's own word: it stops, everyone is muted, it takes a message, a compaction. */
  | { kind: "room"; text: string }
  /** A participant's question settled (`rooms::question_note`, kept in the
   * room's transcript): `[Fable was allowed to write x]`, `[… , and made
   * writable]`, `[Fable was not allowed to write x: why]`, `[Fable no longer asks to write x]`. */
  | { kind: "asked"; outcome: "allowed" | "always" | "declined" | "withdrawn"; name: string; slug: string; text: string };

/** What a note the room streamed says, given its roster (null: not a room's note). */
export function roomNote(text: string, roster: RoomParticipant[]): RoomNote | null {
  const t = text.trim();
  const m = /^\[(.+?) · ([^\]\s]+)\]$/.exec(t);
  if (m) {
    const p = byName(m[1]!, roster);
    if (p || roster.length === 0) return { kind: "speaker", speaker: { slug: p?.slug ?? slugOf(m[1]!), name: m[1]!, thread: p?.thread ?? "", model: m[2]! } };
  }
  const k = /^\[(.+?) (passed|was stopped|said nothing|did not answer: .*)\]$/.exec(t);
  if (k) {
    const p = byName(k[1]!, roster);
    if (p) {
      const v = k[2]!;
      return { kind: v === "passed" ? "pass" : v === "was stopped" ? "stopped" : v === "said nothing" ? "silent" : "failed", name: p.name, slug: p.slug, text: t };
    }
  }
  const a = /^\[(.+?) (was allowed to .*|was not allowed to .*|no longer asks to .*)\]$/s.exec(t);
  if (a) {
    const p = byName(a[1]!, roster);
    if (p) {
      const v = a[2]!;
      const outcome = v.startsWith("was not") ? "declined" : v.startsWith("no longer") ? "withdrawn" : /, and (made writable|from now on: )/.test(v) ? "always" : "allowed";
      return { kind: "asked", outcome, name: p.name, slug: p.slug, text: t };
    }
  }
  if (/^\[(The room|Everyone in the room|The room's round)/.test(t) || /was compacted into .* which speaks from now\]$/.test(t)) return { kind: "room", text: t };
  return null;
}

const byName = (name: string, roster: RoomParticipant[]) => roster.find((p) => p.name === name) ?? roster.find((p) => p.slug === name.toLowerCase());
const slugOf = (name: string) => name.toLowerCase().replace(/[^a-z0-9-]/g, "");

/** A pass note's quiet words: "Gemini passed". */
export function quietNote(n: RoomNote): string {
  switch (n.kind) {
    case "speaker": return `${n.speaker.name} speaks`;
    case "pass": return `${n.name} passed`;
    case "stopped": return `${n.name} was stopped`;
    case "silent": return `${n.name} said nothing`;
    case "failed": return n.text.slice(1, -1);
    case "room": return n.text.replace(/^\[|\]$/g, "");
    case "asked": return n.text.slice(1, -1);
  }
}

// ---- participants' questions (design §2.27: a room shows what its participants ask)

/** A question waiting on the user, as the list's row (`question`) or the
 * attention list (`Question`, which also says what "always" allows) has it. */
export interface PendingQuestion { id: string; command: string; thread?: string | null; grant?: string[] }

/** A participant's question waiting in the room. */
export interface RoomAsk { participant: RoomParticipant; question: PendingQuestion; text: string; what: string | null }

/** The questions the room's participants wait on, in roster order: a
 * participant's row's (its compaction chain's newest thread), else one of
 * the attention list's asked by any thread of its chain. */
export function roomAsks(roster: RoomParticipant[], rows: ThreadRow[], questions: PendingQuestion[] = []): RoomAsk[] {
  const out: RoomAsk[] = [];
  for (const p of roster) {
    const row = rowOf(p, rows);
    const chain = new Set([p.thread, ...(row ? [row.id, ...(row.earlier ?? [])] : [])]);
    const listed = questions.find((q) => q.thread && chain.has(q.thread));
    const q = row?.question ? { ...row.question, thread: row.id, grant: listed?.id === row.question.id ? listed.grant : undefined } : listed;
    if (q) out.push({ participant: p, question: q, ...askText(p, q.command) });
  }
  return out;
}

/** A question's words in a room: a read-only participant's ask, "Fable
 * (read-only, in room “…”) wants to write x", as "Fable wants to write x";
 * a command, "Fable wants to run" with the command apart (`what`). */
export function askText(p: { name: string }, command: string): { text: string; what: string | null } {
  const m = /^(.+?) \(read-only, in room “.*?”\) wants to (.*)$/s.exec(command);
  if (m && m[1] === p.name) return { text: `${p.name} wants to ${m[2]}`, what: null };
  return { text: `${p.name} wants to run`, what: command };
}

/** The working line of a room waiting on its participants: "Fable is
 * waiting for your approval" (and how many others). */
export function waitingLine(asks: RoomAsk[]): string | null {
  if (!asks.length) return null;
  const names = [...new Set(asks.map((a) => a.participant.name))];
  return names.length === 1 ? `${names[0]} is waiting for your approval` : `${names.slice(0, -1).join(", ")} and ${names.at(-1)} are waiting for your approval`;
}

// ---- `@slug` mentions (the composer)

/** The mention being typed at `caret`: an `@` at a word's start, then
 * handle characters up to the caret. */
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const before = text.slice(0, caret);
  const m = /(^|[\s([{,;])@([A-Za-z0-9_.-]*)$/.exec(before);
  if (!m) return null;
  return { start: caret - m[2]!.length - 1, query: m[2]! };
}

/** The participants a typed `@query` may mean: handles first, then names
 * (spaces dropped, as the node matches them), case aside. */
export function mentionMatches(query: string, roster: RoomParticipant[]): RoomParticipant[] {
  const q = query.toLowerCase();
  const name = (p: RoomParticipant) => p.name.toLowerCase().replace(/\s+/g, "");
  const rank = (p: RoomParticipant) =>
    p.slug.startsWith(q) ? 0 : name(p).startsWith(q) ? 1 : p.slug.includes(q) || name(p).includes(q) ? 2 : -1;
  return roster.map((p, i) => ({ p, r: rank(p), i })).filter((x) => x.r >= 0).sort((a, b) => a.r - b.r || a.i - b.i).map((x) => x.p);
}

/** `text` with the mention typed from `start` to `caret` replaced by
 * `@slug ` (no second space when one follows), and where the caret goes. */
export function insertMention(text: string, start: number, caret: number, slug: string): { text: string; caret: number } {
  const after = text.slice(caret);
  const ins = "@" + slug + (after.startsWith(" ") ? "" : " ");
  return { text: text.slice(0, start) + ins + after, caret: start + ins.length + (after.startsWith(" ") ? 1 : 0) };
}

/** The participants a message addresses (`@slug` or `@Name`, as the node
 * schedules them), in mention order; none: everyone unmuted answers. */
export function addressed(text: string, roster: RoomParticipant[]): RoomParticipant[] {
  const out: RoomParticipant[] = [];
  for (const m of text.matchAll(/(?:^|[^A-Za-z0-9_])@([A-Za-z0-9_.-]+)/g)) {
    const h = m[1]!.toLowerCase().replace(/[.]+$/, "");
    const p = roster.find((x) => x.slug === h || x.name.toLowerCase().replace(/\s+/g, "") === h);
    if (p && !out.includes(p)) out.push(p);
  }
  return out;
}

// ---- requests

/** A model to add: read-only by default, or without tools, on `node` when given. */
export interface ModelPick {
  model: string;
  node?: string | null;
  access?: "read-only" | "writable" | "no-tools";
  name?: string | null;
}

export function modelSpec(p: ModelPick): RoomParticipantSpec {
  const access = p.access ?? "read-only";
  return { model: p.model, thread: null, name: p.name ?? null, slug: null, node: p.node || null, tools: access !== "no-tools", read_only: access !== "writable" };
}

/** An existing thread joining as it is. */
export const threadSpec = (thread: string): RoomParticipantSpec => ({ model: null, thread, name: null, slug: null, node: null, tools: true, read_only: false });

export interface NewRoom {
  title?: string | null;
  /** The place's key (`PlaceInfo.key`); none: `from` thread's, else global. */
  place?: string | null;
  /** "Pull in model…": the 1:1 thread the room continues (Participant #1). */
  from?: string | null;
  models?: ModelPick[];
  threads?: string[];
  scopes?: StagedScope[];
  mode?: "sequential" | "parallel";
  hopCap?: number | null;
  budgetUsd?: number | null;
}

export function roomCreate(r: NewRoom): Request {
  return {
    RoomCreate: {
      title: r.title?.trim() || null,
      place: r.from ? null : r.place ?? null,
      from_thread: r.from ?? null,
      participants: [...(r.threads ?? []).filter((t) => t !== r.from).map(threadSpec), ...(r.models ?? []).map(modelSpec)],
      scopes: scopeTargets(r.scopes ?? []),
      mode: r.mode ?? null,
      hop_cap: r.hopCap ?? null,
      budget_usd: r.budgetUsd ?? null,
    },
  };
}

/** How many participants the room would have (the pulled-from thread is one). */
export const participantCount = (r: NewRoom) => (r.from ? 1 : 0) + (r.threads ?? []).filter((t) => t !== r.from).length + (r.models ?? []).length;

export interface RoomChange {
  add?: RoomParticipantSpec[];
  remove?: string[];
  mute?: string[];
  unmute?: string[];
  mode?: "sequential" | "parallel";
  hopCap?: number;
}

export function roomUpdate(room: string, c: RoomChange): Request {
  return { RoomUpdate: { thread: room, add: c.add ?? [], remove: c.remove ?? [], mute: c.mute ?? [], unmute: c.unmute ?? [], mode: c.mode ?? null, hop_cap: c.hopCap ?? null } };
}

/** The room as the node says it, from `Reply::Lines([json])`. */
export function infoOf(r: unknown): RoomInfo {
  if (typeof r !== "object" || r === null || !("Lines" in r)) throw new Error("unexpected reply");
  const line = (r as { Lines: string[] }).Lines[0];
  if (!line) throw new Error("the node said nothing of the room");
  return JSON.parse(line) as RoomInfo;
}

/** The change to `info` a `RoomUpdate` makes, as the client shows it at
 * once (the node's answer replaces it). */
export function applyChange(info: RoomInfo, c: RoomChange): RoomInfo {
  const participants = info.participants
    .filter((p) => !c.remove?.includes(p.slug))
    .map((p) => (c.mute?.includes(p.slug) ? { ...p, muted: true } : c.unmute?.includes(p.slug) ? { ...p, muted: false } : p));
  return { ...info, participants, mode: c.mode ?? info.mode, hop_cap: c.hopCap ?? info.hop_cap };
}

/** Second opinions for "Pull in model…": models of other providers than
 * the thread's first (the first of each), then other models; `n` of them. */
export function secondOpinions(current: string | null, models: string[], n = 1): string[] {
  const own = providerOf(current);
  const others = models.filter((m) => m !== current && !m.startsWith("mock/"));
  const firstOf = new Map<string, string>();
  for (const m of others) if (providerOf(m) !== own && !firstOf.has(providerOf(m))) firstOf.set(providerOf(m), m);
  const pick = [...firstOf.values(), ...others.filter((m) => ![...firstOf.values()].includes(m))];
  return pick.slice(0, n);
}

/** The roster bar's line for a participant: short model, node when not
 * here, spend. */
export function pillMeta(p: RoomParticipant, row: ThreadRow | null, here: string | null): string {
  const bits = [shortModel(p.model)];
  if (row?.node && here && row.node !== here) bits.push("@" + row.node);
  if (row?.cost) bits.push(`$${row.cost.toFixed(2)}`);
  return bits.filter(Boolean).join(" · ");
}
