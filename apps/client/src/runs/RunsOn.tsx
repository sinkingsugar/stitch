import { useEffect } from "preact/hooks";
import type { ThreadItem } from "../model";
import { attention } from "../home/state";
import { refresh } from "../store";
import { loadRuns, runOn, runners, runsError, unreached } from "./state";

/** "Runs on <node>" in the thread header, with the "Continue on…" menu. A
 * thread runs on its owner (where it last stepped) unless the user chose a node
 * or "Run here": the list's `node` is that runner. */
export function RunsOn({ t }: { t: ThreadItem }) {
  useEffect(() => { void loadRuns(t.id); }, [t.id]);
  const here = attention.value?.node;
  const choices = runners.value.filter((r) => r.runs !== false && r.name !== t.node);
  const go = async (e: Event, node: string | null) => {
    (e.currentTarget as HTMLElement).closest("details")!.removeAttribute("open");
    await runOn(t.id, node);
    await refresh();
  };
  return (
    <span class="runs">
      <span class={"meta runson" + (here && t.node !== here ? " pinned" : "")} title="Where this thread's next turn runs; what is typed here is passed on and streams back">
        {here ? (t.node === here ? "Runs here" : `Runs on ${t.node}`) : ""}
      </span>
      <details class="menu">
        <summary>Continue on…</summary>
        <div class="items">
          {choices.map((r) => (
            <button key={r.name} onClick={(e) => void go(e, r.name)}>Continue on {r.name}{unreached(r)}</button>
          ))}
          {here && t.node !== here && <button onClick={(e) => void go(e, null)}>Run here ({here})</button>}
          {!choices.length && (!here || t.node === here) && <span class="meta">No other node runs programs.</span>}
          {runsError.value && <span class="meta err">{runsError.value}</span>}
        </div>
      </details>
    </span>
  );
}
