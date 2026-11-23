// Read state of a conversation (design §2.25, step 1): the node keeps a cursor
// per conversation (`Request::Read`, a synced `set`) and says `unread` in each
// row. This decides, purely, when the client moves it.
import type { ThreadItem } from "./model";

/** The `upto` to send for the open conversation, or null: only when its row is
 * unread, its end is in view and the window is focused, and the cursor would
 * really advance past what this client already sent. `upto` is the row's own
 * `last_step` (what the user was shown), never the clock, so a step that
 * lands between showing and writing stays unread. */
export function readTarget(row: Pick<ThreadItem, "unread" | "last_step">, atEnd: boolean, focused: boolean, sent: string | undefined): string | null {
  if (!row.unread || !atEnd || !focused || !row.last_step) return null;
  if (sent !== undefined && Date.parse(row.last_step) <= Date.parse(sent)) return null;
  return row.last_step;
}

/** The rows a "mark all read" covers, each with its `upto`. */
export function unreadOf(rows: Iterable<ThreadItem>): { id: string; upto: string }[] {
  const out: { id: string; upto: string }[] = [];
  for (const t of rows) if (t.unread && t.last_step) out.push({ id: t.id, upto: t.last_step });
  return out;
}

/** The list with these threads read up to `upto` (a row newer than that stays unread). */
export function readThrough(list: ThreadItem[], done: Map<string, string>): ThreadItem[] {
  return list.map((t) => {
    const upto = done.get(t.id);
    return upto !== undefined && t.unread && Date.parse(t.last_step ?? "") <= Date.parse(upto) ? { ...t, unread: false } : t;
  });
}
