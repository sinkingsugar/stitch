// A message with its hover actions: fork from here, rewind to here, quote, copy.
import * as S from "../store";
import * as A from "./state";
import type { Entry } from "../model";
import { blobUrl, isImage, type Staged } from "../transcript/blob";
import { focusDid } from "../digest/state";

/** Edit & resend, as the TUI and the phone: the thread goes back to the message before
 * this one (what came after is set aside) and the composer holds its text and files. */
async function edit(threadId: string, e: Extract<Entry, { kind: "user" | "assistant" }>) {
  const list = S.entries.value;
  const i = list.indexOf(e);
  let before: string | null = null;
  for (let k = i - 1; k >= 0 && !before; k--) {
    const x = list[k]!;
    if ((x.kind === "user" || x.kind === "assistant") && x.id) before = x.id;
  }
  const files: Staged[] = [...e.images, ...(e.files ?? [])].map((f) => ({ ...f, bytes: (f as { bytes?: number }).bytes ?? 0, preview: isImage(f.type) ? blobUrl(f.hash) : null }));
  const ok = await A.rewind(threadId, before);
  const scopes = (e.scopes ?? []).map((c) => ({ kind: c.kind, id: c.id, title: c.title ?? null, threads: c.threads }));
  if (ok) A.edit.value = { text: e.text, files, scopes };
}

export function Msg({ e, children }: { e: Entry; children: preact.ComponentChildren }) {
  // The row's ID and state alone: a list push re-renders no message.
  const tid = S.currentId.value;
  // An earlier part of a compacted conversation, read as it was: its IDs
  // are its own thread's, not the newest's that forks and rewinds act on.
  const did = (e.kind === "user" || e.kind === "assistant") && e.mid ? e.mid : undefined;
  // Every message carries its durable name (`data-did`), so a digest's item
  // finds it whatever thread of the conversation it is of, shown or not.
  const searchMatch = !!did && A.focusedMid() === did;
  const marked = (c: preact.ComponentChildren) => (did ? <div class={"mdid" + (searchMatch ? " search-focus" : focusDid.value === did ? " focus" : "")} data-did={did}>{c}</div> : <>{c}</>);
  // What a fork continues (design §2.15): read-only here, opened where it is.
  if ((e.kind === "user" || e.kind === "assistant") && e.ancestor) {
    const src = e.source;
    return (
      <div class={"mwrap ancestor" + (searchMatch ? " search-focus" : "") + (e.inherited ? " inctx" : "") + (e.shared ? " shared" : "")} data-did={did}>
        {children}
        <div class="mact">
          {src && <button title="Open the thread this message is of" onClick={() => void S.select(src.thread)}>Open in original</button>}
          <button title="Quote into the message box" onClick={() => A.quote(e.text)}>Quote</button>
          <button title="Copy the text" onClick={() => void A.copy(e.text)}>Copy</button>
        </div>
      </div>
    );
  }
  if (!tid || (e.kind !== "user" && e.kind !== "assistant") || !e.id || S.latest.value) return marked(children);
  const id = e.id;
  // What a fork inherits is its source's: only that thread can fork or rewind there.
  const own = !e.inherited;
  return (
    <div class={"mwrap" + (searchMatch ? " search-focus" : did && focusDid.value === did ? " focus" : "") + (e.inherited ? " inherited" : "") + (e.shared ? " shared" : "")} data-did={did}>
      {children}
      <div class="mact">
        {own && <button title="New thread from here" onClick={() => (A.dialog.value = { k: "fork", upto: id, preset: "fork" })}>Fork</button>}
        {own && <button title="Go on from here in this thread" onClick={() => (A.dialog.value = { k: "rewind", id, text: e.text })}>Rewind</button>}
        {own && e.kind === "user" && !S.currentWorking.value && <button title="Rewind to before this message and edit it" onClick={() => void edit(tid, e)}>Edit</button>}
        <button title="Quote into the message box" onClick={() => A.quote(e.text)}>Quote</button>
        <button title="Copy the text" onClick={() => void A.copy(e.text)}>Copy</button>
      </div>
    </div>
  );
}
