// The model of the next turn, in the composer: a chip with the effective model
// (`Models` + `EffectiveModel`), a menu to pick another for this chat. The pick
// goes with `Chat.model`, as the TUI's does.
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import * as S from "../store";
import * as A from "../actions/state";
import { modelGroups, modelName } from "./modelGroups";

let cached: A.ModelList | null = null;

/** `thread`: the open thread, or null for a new one. */
export function ModelChip({ thread }: { thread: string | null }) {
  const key = thread ?? "new";
  const pick = S.picks.value[key] ?? null;
  const place = thread ? null : S.newPlace.value;
  const [eff, setEff] = useState("");
  const [m, setM] = useState<A.ModelList | null>(cached);
  const [open, setOpen] = useState(false);
  const [provider, setProvider] = useState<string | null>(null);
  const [err, setErr] = useState("");
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const lastGroup = useRef<string | null>(null);
  const groups = modelGroups(m?.models ?? []);
  useEffect(() => {
    let live = true;
    A.effectiveModel(thread, pick, place).then((s) => { if (live) setEff(s); }, () => { if (live) setEff(""); });
    return () => { live = false; };
  }, [thread, pick, place]);
  useEffect(() => {
    if (!open) return;
    if (cached) { setM(cached); return; }
    setErr("");
    A.models().then((x) => setM((cached = x)), (e) => setErr(String(e)));
  }, [open]);
  useLayoutEffect(() => {
    if (!open || !panel.current) return;
    panel.current.scrollTop = 0;
    const buttons = Array.from(panel.current.querySelectorAll("button"));
    const previous = provider === null && lastGroup.current !== null
      ? buttons.find((b) => b.dataset.provider === lastGroup.current) : null;
    (previous ?? buttons[0])?.focus();
  }, [open, provider]);
  const close = () => {
    setOpen(false);
    setProvider(null);
    lastGroup.current = null;
    trigger.current?.focus();
  };
  const row = (v: string | null, label: string, note = "") => (
    <button key={v ?? "auto"} class={"mi" + (pick === v ? " on" : "")} aria-pressed={pick === v} onClick={() => { A.pickModel(key, v); close(); }}>
      <span aria-hidden="true">{pick === v ? "● " : "○ "}</span>{label}{note && <span class="meta">{note}</span>}
    </button>
  );
  const short = modelName(eff);
  return (
    <div class="menu modelchip" onKeyDown={(e) => {
      if (open && e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        if (provider === null) close();
        else setProvider(null);
      }
    }}>
      <button ref={trigger} class={"chipbtn" + (pick ? " picked" : "")} aria-expanded={open} aria-label={`Model: ${short || "node choice"}`} title={`Model for the next turn: ${eff || "…"}${pick ? " (picked for this chat)" : ""}`} onClick={() => open ? close() : setOpen(true)}>
        <span class="mdot" /><span class="chiplabel">{short || "model"}</span> ▾
      </button>
      {open && (
        <>
          <div class="scrim" onClick={close} />
          <div ref={panel} class="mlist up" role="group" aria-label={provider === null ? "Choose a model" : `${provider} models`}>
            {provider === null ? <>
              {row(null, thread ? "Thread's own / node's choice" : "Node's choice")}
              {row("default", "default", "the synced default")}
              {row("quick", "quick", "the quick model")}
              {groups.size > 0 && <hr />}
              {[...groups].map(([name, models]) => (
                <button key={name} data-provider={name} class={"mi modelprovider" + (pick && models.includes(pick) ? " on" : "")} onClick={() => { lastGroup.current = name; setProvider(name); }}>
                  {name}<span aria-hidden="true">›</span>
                </button>
              ))}
              {!m && !err && <div class="meta" style="padding:6px">Loading…</div>}
              {m?.problems.length ? <div class="meta" style="padding:6px">{m.problems.join(" · ")}</div> : null}
              {err && <div class="warn" style="padding:6px">{err}</div>}
            </> : <>
              <button class="mi modelback" onClick={() => setProvider(null)} aria-label="Back to providers">‹ {provider}</button>
              <hr />
              {groups.get(provider)?.map((x) => row(x, modelName(x), x === m?.default ? "default" : ""))}
            </>}
          </div>
        </>
      )}
    </div>
  );
}
