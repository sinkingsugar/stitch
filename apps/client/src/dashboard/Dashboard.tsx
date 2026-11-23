// The built-in dashboard (design §2.10): the client's home view (needs you,
// what's going on, loose ends, scheduled, timeline), rendered on `stitch.live`,
// plus what only the dashboard has: the numbers, and the threads by the
// conversation they came from with their spawn trees, quiet ones folded into
// "Earlier".
import { signal } from "@preact/signals";
import * as H from "../home/state";
import { Dotc, Loose, Needs, Scheduled, TLink, Timeline, Working, useClock } from "../home/Home";
import { threads } from "../home/source";
import { ago, dotOf, lastOf, now, outcomeOf, titleOf, until, type Dot } from "../home/util";
import type { ThreadItem } from "../model";
import { lanesOf, type Lane, type Stack } from "./lanes";
import { DAYS, live, problem } from "./source";

/** A conversation quiet longer than this folds into "Earlier". */
const RECENT = 86400e3;
/** Cards and folded lanes the user opened, kept across renders. */
const opened = signal<Set<string>>(new Set());
const toggle = (k: string) => {
  const s = new Set(opened.value);
  if (!s.delete(k)) s.add(k);
  opened.value = s;
};

const Logo = () => (
  <svg class="logo" viewBox="0 0 1024 1024" aria-hidden="true"><defs>
    <linearGradient id="ib" x1="160" y1="0" x2="864" y2="1024" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#252B3D" /><stop offset="1" stop-color="#131A29" /></linearGradient>
    <linearGradient id="iw" x1="420" y1="218" x2="604" y2="806" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#FFC092" /><stop offset=".48" stop-color="#FF927A" /><stop offset="1" stop-color="#EF706D" /></linearGradient>
    <linearGradient id="ic" x1="330" y1="626" x2="694" y2="398" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#61C5C5" /><stop offset="1" stop-color="#BAF1DC" /></linearGradient>
    <mask id="ix" maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024"><rect width="1024" height="1024" fill="white" /><path d="M326 626 C410 626 446 562 512 512 C578 462 614 398 698 398" fill="none" stroke="black" stroke-width="140" stroke-linecap="round" /></mask></defs>
    <rect width="1024" height="1024" fill="url(#ib)" />
    <path d="M704 286 C624 210 400 214 338 316 C270 428 380 486 512 512 C644 538 754 596 686 708 C624 810 400 814 320 738" fill="none" stroke="url(#iw)" stroke-width="108" stroke-linecap="round" mask="url(#ix)" />
    <path d="M326 626 C410 626 446 562 512 512 C578 462 614 398 698 398" fill="none" stroke="url(#ic)" stroke-width="104" stroke-linecap="round" />
  </svg>
);

function Stats() {
  const ts = threads.value;
  const today = (t: ThreadItem) => now.value - Date.parse(lastOf(t)) < 86400e3;
  const working = ts.filter((t) => t.state === "working").length;
  const failed = ts.filter((t) => dotOf(t) === "failed" && today(t)).length;
  const done = ts.filter((t) => dotOf(t) === "done" && today(t)).length;
  const jobs = H.jobs.value;
  const running = (jobs?.jobs ?? []).filter((j) => j.running).length;
  const active = (H.schedules.value?.schedules ?? []).filter((x) => x.state === "active");
  const soonest = active.find((x) => x.next.length);
  const loose = H.looseItems.value.length;
  const auto = H.loose.value?.auto?.mode;
  const stat = (n: number, label: string, cls: string, href?: string) => (
    <a class={"stat " + (n ? "s-" + cls : "quiet")} href={href}><b>{n}</b><span>{label}</span></a>
  );
  return (
    <div class="stats">
      {stat(H.needsCount.value, "need you", "waiting", "#needs")}
      {stat(working, "working now", "working", "#working")}
      {H.loose.value && stat(loose, `loose end${loose === 1 ? "" : "s"}${auto && auto !== "off" ? ` · auto ${auto === "on" ? "on" : "dry run"}` : ""}`, "waiting", "#loose")}
      {stat(failed, "failed today", "failed", "#threads")}
      {stat(done, "done today", "done", "#threads")}
      {stat(running, `job${running === 1 ? "" : "s"}${jobs ? " on " + jobs.node : ""}`, "working", "#working")}
      {H.schedules.value && stat(active.length, `scheduled${soonest ? " · next " + until(soonest.next[0]!) : ""}`, "sched", "#scheduled")}
    </div>
  );
}

function Card({ s, main }: { s: Stack; main: string | undefined }) {
  const t = s.t;
  const state: Dot = dotOf(t);
  const open = opened.value.has(t.id);
  const facts = [ago(lastOf(t)), t.model_calls ? `${t.model_calls} calls` : "", t.cost ? `$${t.cost.toFixed(2)}` : ""].filter(Boolean).join(" · ");
  return (
    <div class="stack">
      <div class={"tc s-" + state + (open ? " open" : "")} title={t.id}
        onClick={(e) => { if (!(e.target as HTMLElement).closest("pre, a")) toggle(t.id); }}>
        <div class="tl"><Dotc c={state} /><div class="name">{titleOf(t)}</div></div>
        <div class="out">{t.question ? <>waits for you to run <code>{t.question.command}</code></> : outcomeOf(t)}</div>
        <div class="cmeta">
          <span class="chip node">{t.node}</span>
          {t.place !== main && <span class="chip">{t.place}</span>}
          {t.read_only && <span class="chip ro">read-only</span>}
          <span>{facts}</span>
        </div>
        {open && (
          <div class="more">
            <div class="meta">Goal</div><pre>{t.goal}</pre>
            {t.last_words && <><div class="meta">Last words</div><pre>{t.last_words}</pre></>}
            <div class="meta">
              {t.id}{t.started_on && t.started_on !== t.node ? ` · started on ${t.started_on}` : ""} · started {ago(t.started)}
              {" "}<TLink id={t.id}>Open</TLink>
            </div>
          </div>
        )}
      </div>
      {s.kids.length > 0 && <div class="kids">{s.kids.map((k) => <Card key={k.t.id} s={k} main={main} />)}</div>}
    </div>
  );
}

function LaneHead({ l }: { l: Lane }) {
  const counts = (["working", "waiting", "failed", "done"] as Dot[]).filter((k) => l.counts[k]);
  return (
    <div class="lane-h">
      <b>{l.conversation ? <TLink id={l.conversation}>{l.title}</TLink> : l.title}</b>
      <span class="meta">{[l.places.join(", "), l.nodes.join(", "), ago(new Date(l.latest).toISOString())].filter(Boolean).join(" · ")}</span>
      <span class="counts">{counts.map((k) => <span key={k}><Dotc c={k} />{l.counts[k]}</span>)}</span>
    </div>
  );
}

function LaneView({ l, folded }: { l: Lane; folded: boolean }) {
  const body = <div class="cards">{l.roots.map((s) => <Card key={s.t.id} s={s} main={l.places[0]} />)}</div>;
  if (!folded) {
    // As wide as its cards in a row: small lanes share a line.
    const wide = Math.min(l.roots.length, 4) * 282 + 34;
    return <section class="lane" style={{ flexBasis: `min(100%, ${wide}px)` }}><LaneHead l={l} />{body}</section>;
  }
  const k = "lane:" + l.key;
  return (
    <details class="lane folded" open={opened.value.has(k)} onToggle={(e) => { if ((e.currentTarget as HTMLDetailsElement).open !== opened.value.has(k)) toggle(k); }}>
      <summary><LaneHead l={l} /></summary>
      {opened.value.has(k) && body}
    </details>
  );
}

function Threads() {
  const lanes = lanesOf(threads.value);
  const recent = lanes.filter((l) => l.live || now.value - l.latest < RECENT);
  const older = lanes.filter((l) => !recent.includes(l));
  return (
    <>
      <div class="dhead" id="threads">
        <h2>Threads</h2><span class="meta">by the conversation that started them</span>
        <span class="legend"><span><Dotc c="working" />working</span><span><Dotc c="waiting" />waits on you</span><span><Dotc c="done" />done</span><span><Dotc c="failed" />failed</span></span>
      </div>
      <div class="lanes">{recent.length ? recent.map((l) => <LaneView key={l.key} l={l} folded={false} />) : <div class="meta quiet">Nothing moved in the last day.</div>}</div>
      {older.length > 0 && (
        <>
          <div class="dhead"><h2>Earlier</h2><span class="meta">{older.length} conversation{older.length > 1 ? "s" : ""} quiet for a day or more, of the last {DAYS} days</span></div>
          <div class="earlier">{older.map((l) => <LaneView key={l.key} l={l} folded />)}</div>
        </>
      )}
    </>
  );
}

export function Dashboard() {
  useClock();
  const p = problem.value;
  const lv = live.value;
  return (
    <div class="dash">
      <header class="dtop">
        <Logo />
        <h1><span>Stitch</span> dashboard</h1>
        <span class={"live" + (lv.ok ? "" : " off")}><i />{lv.ok ? `live · ${lv.at!.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : lv.at ? "not live" : "connecting…"}</span>
      </header>
      {p && <div class="panel ask problem">{p.consent && <b>This dashboard needs your permission</b>}<div>{p.text}</div></div>}
      {!p?.consent && (
        <>
          {H.liveError.value && <div class="err" onClick={() => (H.liveError.value = null)}>{H.liveError.value}</div>}
          <Stats />
          <div class="front">
            <div class="col">
              <div class="o1" id="needs"><Needs /></div>
              <div class="o3" id="loose"><Loose /></div>
            </div>
            <div class="col">
              <div class="o2" id="working"><Working /></div>
              <div class="o4" id="scheduled"><Scheduled /></div>
            </div>
          </div>
          <Threads />
          <div class="tlwrap"><Timeline /></div>
        </>
      )}
    </div>
  );
}
