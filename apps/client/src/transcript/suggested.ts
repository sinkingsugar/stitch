// Preset answers (design §2.30, "Inline controls", step 1): a
// `suggest_replies` call's `replies` are buttons at the end of its turn.
// Live while it is the thread's latest set (no user message after it, no
// turn running): pressing one sends its text as a message, as typed. Once
// answered, the one picked shows marked; a set never picked disappears.
import type { Entry } from "../model";
import type { ActionButton, SuggestedReply } from "../payload";

export interface ReplyBlock {
  replies: SuggestedReply[];
  live: boolean;
  /** The reply whose text the next user message is (pressed, or typed the
   * same), else null. */
  picked: number | null;
}

const isUser = (e: Entry) => e.kind === "user";
const repliesOf = (e: Entry) => (e.kind === "tool" && e.replies?.length ? e.replies : null);

/** Where each set of preset answers shows: by the index of the entry it goes
 * after (its turn's last), what it offers and whether it is live. A later
 * set in the same turn replaces an earlier one; while a turn runs, none
 * is live (nor is the running turn's shown yet). */
export function replyBlocks(entries: Entry[], running: boolean): Map<number, ReplyBlock> {
  const out = new Map<number, ReplyBlock>();
  let pending: { replies: SuggestedReply[] } | null = null;
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i]!;
    const r = repliesOf(e);
    if (r) pending = { replies: r };
    if (isUser(e) && pending) {
      const said = e.kind === "user" ? e.text.trim() : "";
      const picked = pending.replies.findIndex((x) => x.text.trim() === said);
      // Answered by one of them: shown, marked, before the message.
      if (picked >= 0 && i > 0) out.set(i - 1, { replies: pending.replies, live: false, picked });
      pending = null;
    }
  }
  if (pending && !running && entries.length) out.set(entries.length - 1, { replies: pending.replies, live: true, picked: null });
  return out;
}

// Action buttons (design §2.30, step 2): an `offer_actions` call's
// `actions`, shown at the end of its turn. The thread's latest set is live,
// whatever came after it (the user writing, turns, presses): each press
// runs its call on the node. Older sets stay, inert. Each button shows how
// its latest press ended.

export interface ActionBlock {
  set: string;
  actions: ActionButton[];
  live: boolean;
  /** Per button: its latest press ok, failed, or never pressed (null). */
  last: (boolean | null)[];
}

/** Where each action set shows (by the index of the entry it goes after:
 * its turn's last, before the next message of the user's, presses after it
 * not counted), and how. */
export function actionBlocks(entries: Entry[]): Map<number, ActionBlock> {
  const out = new Map<number, ActionBlock>();
  const bySet = new Map<string, ActionBlock>();
  let pending: ActionBlock | null = null;
  let latest: ActionBlock | null = null;
  const place = (at: number) => {
    if (pending && at >= 0) out.set(at, pending);
    pending = null;
  };
  // The last entry of the turn so far: presses come after it, outside it.
  let end = -1;
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i]!;
    if (e.kind === "press") {
      const b = bySet.get(e.set);
      if (b && e.item < b.last.length) b.last[e.item] = e.ok;
      continue;
    }
    if (e.kind === "tool" && e.actions?.length && e.set && e.result?.ok !== false) {
      if (pending) place(end);
      const b: ActionBlock = { set: e.set, actions: e.actions, live: false, last: e.actions.map(() => null) };
      bySet.set(b.set, b);
      pending = latest = b;
    } else if (isUser(e)) {
      place(end);
    }
    end = i;
  }
  place(end);
  if (latest) (latest as ActionBlock).live = true;
  return out;
}
