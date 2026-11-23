import { signal } from "@preact/signals";
import * as S from "../store";
import * as H from "../home/state";
import { Job } from "../home/Home";
import { ago, dotOf, money, titleOf, sentence } from "../home/util";

export const panelOpen = signal(localStorage.getItem("stitch.panel") !== "0");
export const togglePanel = () => {
  panelOpen.value = !panelOpen.value;
  localStorage.setItem("stitch.panel", panelOpen.value ? "1" : "0");
};

function Kid({ n, depth }: { n: H.TreeNode; depth: number }) {
  const t = n.t;
  return (
    <>
      <button class={"kid" + (S.selected.value === t.id ? " sel" : "")} style={`margin-left:${depth * 12}px`} onClick={() => void S.select(t.id)}>
        <span class={"dot " + dotOf(t)} />
        <span class="rt">
          <span class="title">{titleOf(t)}</span>
          <span class="meta">{t.state}{t.outcome ? ` · ${t.outcome.replace(/-/g, " ")}` : ""}{t.cost ? ` · ${money(t.cost)}` : ""} · {ago(t.last_step)}</span>
          {t.last_words && <span class="words2">{sentence(t.last_words, 140)}</span>}
        </span>
      </button>
      {n.kids.map((k) => <Kid key={k.t.id} n={k} depth={depth + 1} />)}
    </>
  );
}

/** A narrow right panel: what this thread spawned, its jobs, the tree's cost. */
export function SpawnPanel() {
  const id = S.selected.value;
  const n = id ? H.nodeOf.value.get(id) : undefined;
  if (!n) return null;
  const jobs = (H.jobs.value?.jobs ?? []).filter((j) => j.thread === id);
  if (!panelOpen.value) return <button class="panel-tab" title="Show spawned threads" onClick={togglePanel}>◂ {n.kids.length || ""}</button>;
  const total = H.treeCost(n);
  let count = -1;
  H.walk(n, () => count++);
  return (
    <aside class="spawn">
      <div class="phead"><h2>Spawned</h2><span class="sp" /><button title="Hide" onClick={togglePanel}>▸</button></div>
      <div class="meta">{count} thread{count === 1 ? "" : "s"} · tree cost {money(total) || "$0"}</div>
      {n.kids.length ? n.kids.map((k) => <Kid key={k.t.id} n={k} depth={0} />) : <div class="meta quiet">Nothing spawned.</div>}
      <div class="phead"><h2>Jobs</h2></div>
      {jobs.length ? jobs.map((j, i) => <Job key={i} j={j} />) : <div class="meta quiet">No jobs.</div>}
    </aside>
  );
}
