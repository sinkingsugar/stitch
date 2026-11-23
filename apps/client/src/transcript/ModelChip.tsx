// The model of the next turn, in the composer: a chip with the effective model
// (`Models` + `EffectiveModel`), a menu to pick another for this chat. The pick
// goes with `Chat.model`, as the TUI's does.
import { useEffect, useState } from "preact/hooks";
import * as S from "../store";
import * as A from "../actions/state";

let cached: A.ModelList | null = null;

/** `thread`: the open thread, or null for a new one. */
export function ModelChip({ thread }: { thread: string | null }) {
  const key = thread ?? "new";
  const pick = S.picks.value[key] ?? null;
  const place = thread ? null : S.newPlace.value;
  const [eff, setEff] = useState("");
  const [m, setM] = useState<A.ModelList | null>(cached);
  const [open, setOpen] = useState(false);
  const [err, setErr] = useState("");
  useEffect(() => { A.effectiveModel(thread, pick, place).then(setEff, () => setEff("")); }, [thread, pick, place]);
  useEffect(() => {
    if (open && !cached) A.models().then((x) => setM((cached = x)), (e) => setErr(String(e)));
  }, [open]);
  const row = (v: string | null, label: string, note = "") => (
    <button key={label} class={"mi" + (pick === v ? " on" : "")} onClick={() => { A.pickModel(key, v); setOpen(false); }}>
      <span>{pick === v ? "● " : "○ "}{label}</span> <span class="meta">{note}</span>
    </button>
  );
  const short = eff.includes("/") ? eff.split("/").slice(1).join("/") : eff;
  return (
    <div class="menu modelchip">
      <button class={"chipbtn" + (pick ? " picked" : "")} title={`Model for the next turn: ${eff || "…"}${pick ? " (picked for this chat)" : ""}`} onClick={() => setOpen(!open)}>
        <span class="mdot" /> {short || "model"} ▾
      </button>
      {open && (
        <>
          <div class="scrim" onClick={() => setOpen(false)} />
          <div class="mlist up">
            {thread ? row(null, "Thread's own / node's choice") : row(null, "Node's choice")}
            {row("default", "default", "the synced default")}
            {row("quick", "quick", "the quick model")}
            <hr />
            {(m?.models ?? []).map((x) => row(x, x, x === m!.default ? "default" : ""))}
            {!m && !err && <div class="meta" style="padding:6px">Loading…</div>}
            {m?.problems.length ? <div class="meta" style="padding:6px">{m.problems.join(" · ")}</div> : null}
            {err && <div class="warn" style="padding:6px">{err}</div>}
          </div>
        </>
      )}
    </div>
  );
}
