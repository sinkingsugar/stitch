// The pure parts of the digest views (design §2.25): the order items are
// shown in, the counts line of the folded block, how a reply's outcome reads.
import type { Digest, DigestItem, DigestKind } from "../payload";

/** The order the unfolded block groups its items in: what needs the user first. */
export const KIND_ORDER: DigestKind[] = ["ask", "question", "outcome", "decision", "finding", "change", "goal"];

export const KIND_LABEL: Record<DigestKind, string> = {
  ask: "Asks you",
  question: "Open",
  outcome: "Outcome",
  decision: "Decisions",
  finding: "Findings",
  change: "Changes",
  goal: "Goal",
};

/** The items grouped by kind in `KIND_ORDER`, each group in the order given (most important first); empty groups left out. */
export function grouped(items: DigestItem[]): { kind: DigestKind; items: DigestItem[] }[] {
  return KIND_ORDER.map((kind) => ({ kind, items: items.filter((i) => i.kind === kind) })).filter((g) => g.items.length > 0);
}

const plural = (n: number, one: string, many = one + "s") => `${n} ${n === 1 ? one : many}`;

/** "1 ask · 2 open · 3 decisions": what the folded block counts, asks first; the goal is not counted. */
export function counts(items: DigestItem[]): string {
  const n = (k: DigestKind) => items.filter((i) => i.kind === k).length;
  const parts: string[] = [];
  if (n("ask")) parts.push(plural(n("ask"), "ask"));
  if (n("question")) parts.push(`${n("question")} open`);
  if (n("decision")) parts.push(plural(n("decision"), "decision"));
  if (n("finding")) parts.push(plural(n("finding"), "finding"));
  if (n("change")) parts.push(plural(n("change"), "change"));
  if (n("outcome")) parts.push(plural(n("outcome"), "outcome"));
  return parts.join(" · ");
}

/** A row's tooltip for its asks mark. */
export const asksTitle = (n: number) => `asks you ${plural(n, "thing")}`;

/** What an update's outcome says, in a line. */
export function said(u: NonNullable<Digest["update"]>): string {
  const cost = `$${u.usd.toFixed(4)}`;
  switch (u.outcome) {
    case "written": return `Updated (${cost}${u.windows > 1 ? `, ${u.windows} windows` : ""}${u.more ? ", more to read later" : ""}).`;
    case "nothing new": return "Nothing new to digest.";
    case "capped": return `Today's cap is reached ($${u.day_usd.toFixed(2)} of $${u.cap_usd.toFixed(2)}): no update.`;
    case "busy": return "An update is already running.";
    default: return `The update failed${u.why ? `: ${u.why}` : ""}.`;
  }
}

/** Whether an item's message is in the open conversation (its chain's threads) or needs its thread opened first. */
export function inConversation(item: DigestItem, row: { id: string; earlier?: string[] } | null): boolean {
  return !!row && (row.id === item.thread || !!row.earlier?.includes(item.thread));
}

/** Whether the block starts unfolded: when the digest holds an ask, or the conversation was unread when it opened. */
export function unfoldByDefault(items: DigestItem[], unread: boolean): boolean {
  return unread || items.some((i) => i.kind === "ask");
}

/** The row's counts line from its digest (the row has asks and questions only). */
export function rowCounts(d: { asks: number; questions: number }): string {
  const parts: string[] = [];
  if (d.asks) parts.push(plural(d.asks, "ask"));
  if (d.questions) parts.push(`${d.questions} open`);
  return parts.join(" · ");
}
