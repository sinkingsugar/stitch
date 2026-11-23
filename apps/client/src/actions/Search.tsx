// Global message search in the sidebar (Request::Search).
import { signal } from "@preact/signals";
import type { SearchHit } from "../api/SearchHit";
import * as A from "./state";

const hits = signal<SearchHit[] | null>(null);
const asking = signal(false);
const q = signal("");

async function go() {
  const v = q.value.trim();
  if (!v) { hits.value = null; return; }
  asking.value = true;
  hits.value = (await A.run("searching", () => A.search(v))) ?? [];
  asking.value = false;
}

export function SearchBox() {
  return (
    <div class="gsearch">
      <input
        placeholder="Search messages (Enter)" value={q.value}
        onInput={(e) => { q.value = e.currentTarget.value; if (!q.value) hits.value = null; }}
        onKeyDown={(e) => e.key === "Enter" && void go()}
      />
      {hits.value && (
        <div class="hits">
          <div class="meta hhead">{hits.value.length} hit{hits.value.length === 1 ? "" : "s"} <button class="link" onClick={() => { hits.value = null; q.value = ""; }}>clear</button></div>
          {hits.value.map((h, i) => (
            <button key={i} class="hit" onClick={() => void A.openHit(h)}>
              <b>{h.title}</b>
              <span class="meta">{h.msg} · {h.sender} · {h.found_by}</span>
              <span class="snip">{h.snippet}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
