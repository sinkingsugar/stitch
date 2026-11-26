// The composer: text, and files (any type) pasted, dropped or picked, uploaded at
// once (`POST /_stitch/blob`) and sent with the message as attachments; the
// model of the turn next to Send; threads and efforts attached as scopes
// (design §2.27), chips above the text, the ⎇ picker next to 📎.
import { useEffect, useRef, useState } from "preact/hooks";
import * as S from "../store";
import { draft, edit, pendingScopes } from "../actions/state";
import { addScopes, scopePrompt, toggleScope, type StagedScope } from "../scopes";
import { ScopePicker, StagedScopes } from "./ScopePicker";
import { upload, isImage, sizeText, type Staged } from "./blob";
import { ModelChip } from "./ModelChip";
import type { RoomParticipant } from "../payload";
import { addressed, insertMention, mentionAt, mentionMatches, shortModel, speakerColor } from "../room/logic";
import { mention } from "../room/state";

/** In a room (design §2.27): who the message goes to, in place of the model chip. */
function RoomTarget({ text, room }: { text: string; room: RoomParticipant[] }) {
  const to = addressed(text, room);
  const all = room.filter((p) => !p.muted);
  return (
    <span class="rtarget" title={to.length ? "Only those named answer, in that order" : "Everyone unmuted answers, in roster order (type @ to address someone)"}>
      → {to.length ? to.map((p, i) => <b key={p.slug} style={`color:${speakerColor(p.slug)}`}>{i ? ", " : ""}@{p.slug}</b>) : <b>{all.length === room.length ? "everyone" : `${all.length} unmuted`}</b>}
    </span>
  );
}

export function RichComposer({ onSend, placeholder, thread, room }: { onSend: (t: string, files: Staged[], scopes: StagedScope[]) => void; placeholder: string; thread: string | null; room?: RoomParticipant[] | null }) {
  const [text, setText] = useState("");
  /** The `@` being typed in a room, and the highlighted match. */
  const [at, setAt] = useState<{ start: number; query: string; i: number } | null>(null);
  const [staged, setStaged] = useState<Staged[]>([]);
  const [scopes, setScopes] = useState<StagedScope[]>([]);
  const [busy, setBusy] = useState(0);
  const [err, setErr] = useState<string | null>(null);
  const [over, setOver] = useState(false);
  const pick = useRef<HTMLInputElement>(null);
  const area = useRef<HTMLTextAreaElement>(null);
  // A quote from a message's action arrives as a draft to append.
  useEffect(() => {
    const d = draft.value;
    if (d == null) return;
    draft.value = null;
    setText((t) => (t ? t.replace(/\n*$/, "\n\n") : "") + d);
    area.current?.focus();
  }, [draft.value]);
  // Scopes pushed from elsewhere (the map, the thread menu) for this composer.
  useEffect(() => {
    const p = pendingScopes.value;
    if (!p) return;
    const mine = p.to === null ? thread === null : thread !== null && (p.to === thread || !!S.current.peek()?.earlier?.includes(p.to));
    if (!mine) return;
    pendingScopes.value = null;
    setScopes((l) => addScopes(l, p.scopes));
    area.current?.focus();
  }, [pendingScopes.value, thread]);
  // A pill's `@slug` (the roster bar): inserted at the caret.
  useEffect(() => {
    const slug = mention.value;
    if (!slug || !room) return;
    mention.value = null;
    const el = area.current;
    const caret = el ? el.selectionStart ?? text.length : text.length;
    const pre = text.slice(0, caret) + (caret > 0 && !/\s$/.test(text.slice(0, caret)) ? " " : "");
    const r = insertMention(pre + "@" + text.slice(caret), pre.length, pre.length + 1, slug);
    setText(r.text);
    setAt(null);
    setTimeout(() => { el?.focus(); el?.setSelectionRange(r.caret, r.caret); }, 0);
  }, [mention.value, !!room]);
  const matches = room && at ? mentionMatches(at.query, room) : [];
  const choose = (slug: string) => {
    const el = area.current;
    if (!at || !el) return;
    const r = insertMention(text, at.start, el.selectionStart ?? text.length, slug);
    setText(r.text);
    setAt(null);
    setTimeout(() => { el.focus(); el.setSelectionRange(r.caret, r.caret); }, 0);
  };
  const track = (el: HTMLTextAreaElement) => {
    if (!room) return;
    const m = mentionAt(el.value, el.selectionStart ?? el.value.length);
    setAt(m && mentionMatches(m.query, room).length ? { ...m, i: at && at.start === m.start ? at.i : 0 } : null);
  };
  // An edit & resend replaces what is there.
  useEffect(() => {
    const e = edit.value;
    if (!e) return;
    edit.value = null;
    setText(e.text);
    setStaged(e.files);
    setScopes(e.scopes ?? []);
    area.current?.focus();
  }, [edit.value]);

  const add = async (files: File[]) => {
    if (!files.length) return;
    setBusy((n) => n + files.length);
    for (const f of files) {
      try {
        const s = await upload(f, f.name || null);
        setStaged((l) => [...l, s]);
        setErr(null);
      } catch (e) {
        setErr(e instanceof Error ? e.message : String(e));
      }
      setBusy((n) => n - 1);
    }
  };
  const go = () => {
    const v = text.trim();
    if ((!v && !staged.length && !scopes.length) || busy) return;
    setText("");
    setStaged([]);
    setScopes([]);
    // Scopes alone: a question about them, so the model has something to answer.
    onSend(v || (scopes.length && !staged.length ? scopePrompt(scopes) : v), staged, scopes);
  };
  return (
    <div
      class={"composer" + (over ? " over" : "")}
      onDragOver={(e) => { if (e.dataTransfer?.types.includes("Files")) { e.preventDefault(); setOver(true); } }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => { e.preventDefault(); setOver(false); void add([...(e.dataTransfer?.files ?? [])]); }}
    >
      <div class="cbox">
        <StagedScopes scopes={scopes} onRemove={(s) => setScopes((l) => l.filter((x) => x !== s))} />
        {(staged.length > 0 || busy > 0 || err) && (
          <div class="staged">
            {staged.map((s, i) => (
              <span class={"stage" + (s.preview && isImage(s.type) ? "" : " file")} key={s.hash + i}>
                {s.preview && isImage(s.type) ? <img src={s.preview} alt={s.name ?? "image"} /> : <span class="fchip">📄 {s.name ?? "file"}{s.bytes ? <span class="meta"> {sizeText(s.bytes)}</span> : null}</span>}
                <button title="Remove" onClick={() => setStaged(staged.filter((x) => x !== s))}>✕</button>
              </span>
            ))}
            {busy > 0 && <span class="stage wait">uploading…</span>}
            {err && <span class="uerr">{err}</span>}
          </div>
        )}
        {at && matches.length > 0 && (
          <div class="mentions" role="listbox" aria-label="Address a participant">
            {matches.map((p, i) => (
              <button key={p.slug} role="option" aria-selected={i === at.i} class={i === at.i ? "on" : ""} style={`--sc:${speakerColor(p.slug)}`}
                onMouseDown={(e) => { e.preventDefault(); choose(p.slug); }}>
                <i class="sdot" aria-hidden="true" /><span class="ms">@{p.slug}</span> {p.name}<span class="meta">{shortModel(p.model)}{p.muted ? " · muted" : ""}</span>
              </button>
            ))}
          </div>
        )}
        <textarea
          ref={area} rows={2} value={text} placeholder={scopes.length ? "Ask about the attached… (Enter sends a briefing request)" : placeholder + (room ? " (@ to address someone)" : " (paste or drop files)")}
          onInput={(e) => { setText(e.currentTarget.value); track(e.currentTarget); }}
          onClick={(e) => track(e.currentTarget)}
          onBlur={() => setAt(null)}
          onPaste={(e) => {
            const files = [...(e.clipboardData?.files ?? [])];
            if (files.length) { e.preventDefault(); void add(files); }
          }}
          onKeyDown={(e) => {
            if (at && matches.length) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); const d = e.key === "ArrowDown" ? 1 : -1; setAt({ ...at, i: (at.i + d + matches.length) % matches.length }); return; }
              if ((e.key === "Enter" || e.key === "Tab") && !e.isComposing) { e.preventDefault(); choose(matches[Math.min(at.i, matches.length - 1)]!.slug); return; }
              if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); setAt(null); return; }
            }
            if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); go(); }
          }}
        />
      </div>
      <input ref={pick} type="file" multiple hidden onChange={(e) => { void add([...(e.currentTarget.files ?? [])]); e.currentTarget.value = ""; }} />
      <button title="Attach files" onClick={() => pick.current?.click()}>📎</button>
      <ScopePicker scopes={scopes} onToggle={(s) => setScopes((l) => toggleScope(l, s))} />
      {room ? <RoomTarget text={text} room={room} /> : <ModelChip key={thread ?? "new"} thread={thread} />}
      <button class="primary" disabled={busy > 0} onClick={go}>Send</button>
    </div>
  );
}
