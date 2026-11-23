// The digest block at the top of a conversation (design §2.25): folded to the
// headline and counts, unfolded to the items by kind, each opening the
// message it comes from; with its model, cost, age and an Update button.
import { useRef, useState } from "preact/hooks";
import * as S from "../store";
import { counts, grouped, KIND_LABEL, unfoldByDefault } from "./logic";
import { digest, focusDid, openItem, outcome, update, updating } from "./state";
import { ago } from "../home/util";

export function DigestBlock({ thread }: { thread: string }) {
  // Whether it was unread when the conversation opened (viewing marks it read).
  const wasUnread = useRef(!!S.current.value?.unread);
  const [chosen, setOpen] = useState<boolean | null>(typeof location !== "undefined" && new URLSearchParams(location.search).has("unfold") ? true : null);
  const d = digest.value?.digest;
  if (!d) return null;
  const open = chosen ?? unfoldByDefault(d.items, wasUnread.current);
  const line = counts(d.items);
  return (
    <section class={"digest" + (d.fresh ? "" : " stale") + (open ? " open" : "")} aria-label="Digest">
      <button class="dhead" aria-expanded={open} title={open ? "Fold the digest" : "Unfold the digest"} onClick={() => setOpen(!open)}>
        <span class="fold">{open ? "▾" : "▸"}</span>
        <span class="dtitle">{d.headline}</span>
        {line && <span class="dcount">{line}</span>}
        {!d.fresh && <span class="dstale" title="The conversation moved since this digest was written">stale</span>}
      </button>
      {open && (
        <div class="dbody">
          {grouped(d.items).map((g) => (
            <div class={"dgroup k-" + g.kind} key={g.kind}>
              <div class="dkind">{KIND_LABEL[g.kind]}</div>
              <ul>
                {g.items.map((i) => (
                  <li key={i.at + i.text} class={focusDid.value === i.at ? "focus" : ""}>
                    <button class="ditem" title={`Open the message it comes from (${i.at})`} onClick={() => void openItem(i)}>{i.text}</button>
                  </li>
                ))}
              </ul>
            </div>
          ))}
          {d.items.length === 0 && <div class="meta">Nothing worth listing yet.</div>}
          <div class="dfoot meta">
            <span>{d.model || "model unknown"}</span>
            <span>${d.usd.toFixed(4)}</span>
            <span title={d.at}>as of {ago(d.at)}</span>
            {!d.fresh && <span class="dstale">stale: the conversation moved since</span>}
            <button class="dupdate" disabled={updating.value} onClick={() => void update(thread)}>{updating.value ? "Updating…" : "Update"}</button>
            {outcome.value && <span class="dout">{outcome.value}</span>}
          </div>
        </div>
      )}
    </section>
  );
}
