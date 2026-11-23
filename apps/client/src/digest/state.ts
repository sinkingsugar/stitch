// The open conversation's digest: read when it opens and again when its
// row's digest changes (`digest.at`), updated on request. Not persisted: the
// fold state is the block's own.
import { effect, signal } from "@preact/signals";
import * as S from "../store";
import type { Digest, DigestItem } from "../payload";
import { inConversation, said } from "./logic";

/** The open conversation's digest reply (`digest: null` when it has none); null until read. */
export const digest = signal<Digest | null>(null);
/** An update is running. */
export const updating = signal(false);
/** What the last update said (cleared when the conversation changes). */
export const outcome = signal<string | null>(null);
/** A message (`<thread>#<id>`) to mark after a digest item opened it. */
export const focusDid = signal<string | null>(null);

let loadedFor: string | null = null;

async function read(thread: string): Promise<Digest | null> {
  const r = await S.rpc({ Digest: { thread, update: false } });
  if (typeof r !== "object" || !("Lines" in r)) throw new Error("unexpected reply to Digest");
  return JSON.parse(r.Lines[0]!) as Digest;
}

/** Reads the digest of `thread` into `digest` (unless another conversation opened meanwhile). */
export async function load(thread: string) {
  try {
    const d = await read(thread);
    if (S.current.value?.id === thread || S.selected.value === thread) digest.value = d;
  } catch {
    // A node without digests: none shown.
  }
}

/** One update now (`Digest { update: true }`): its outcome shown, the digest read again. */
export async function update(thread: string) {
  updating.value = true;
  outcome.value = null;
  try {
    const r = await S.rpc({ Digest: { thread, update: true } });
    if (typeof r !== "object" || !("Lines" in r)) throw new Error("unexpected reply to Digest");
    const d = JSON.parse(r.Lines[0]!) as Digest;
    if (d.update) outcome.value = said(d.update);
    if (S.current.value?.id === thread || S.selected.value === thread) digest.value = d;
    void S.refresh();
  } catch (e) {
    outcome.value = e instanceof Error ? e.message : String(e);
  } finally {
    updating.value = false;
  }
}

// The conversation opened, or its row's digest changed: read it.
effect(() => {
  const row = S.current.value;
  const key = row ? `${row.id}@${row.digest?.at ?? ""}` : null;
  if (!row || S.homeView.value) {
    loadedFor = null;
    digest.value = null;
    return;
  }
  if (key === loadedFor) return;
  if (loadedFor?.split("@")[0] !== row.id) {
    digest.value = null;
    outcome.value = null;
  }
  loadedFor = key;
  void load(row.id);
});

/** Opens the message a digest item comes from: its thread first when it is
 * not of the open conversation, then scrolls to the message and marks it. */
export async function openItem(item: DigestItem) {
  if (!inConversation(item, S.current.value)) await S.select(item.thread);
  focusDid.value = item.at;
  // Earlier than the transcript's window: read back to it.
  await S.reveal((e) => "mid" in e && e.mid === item.at);
  const find = () => document.querySelector(`[data-did="${CSS.escape(item.at)}"]`);
  // The transcript may still be reading: look for it for a few seconds.
  for (let i = 0; i < 30; i++) {
    const el = find();
    if (el) {
      el.scrollIntoView({ block: "center" });
      break;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  setTimeout(() => { if (focusDid.value === item.at) focusDid.value = null; }, 4000);
}
