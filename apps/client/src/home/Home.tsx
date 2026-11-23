import { useEffect, useState } from "preact/hooks";
import { canOpen, src, threads } from "./source";
import type { ThreadItem } from "../model";
import * as H from "./state";
import type { AutoMode, LiveJob, LooseAuto, Schedule, Question } from "./payload";
import { ago, dayOf, dotOf, hhmm, lastOf, outcomeOf, sentence, spoken, titleOf, until, useTick, when, now, type Dot } from "./util";

export const Dotc = ({ c }: { c: Dot | "paused" | "sched" }) => <i class={"dot " + c} />;
const GOOD = /exit 0|matched|quiet/;

export function useClock() { useEffect(() => useTick(), []); void now.value; }

function open(id: string | null) { if (id) src.open?.(id); }

/** A thread's title: a link that opens it where the host can (the client
 * selects it, the phone's dashboard follows `stitch://thread`), else text. */
export function TLink({ id, children }: { id: string | null | undefined; children: preact.ComponentChildren }) {
  if (id && src.open) return <a class="link" onClick={() => open(id)}>{children}</a>;
  const href = id ? src.href?.(id) : null;
  if (href) return <a class="link" href={href}>{children}</a>;
  return <b class="tname">{children}</b>;
}
/** A command to run instead of a button the host does not have. */
const Cmd = ({ children }: { children: string }) => <code class="cmd">{children}</code>;

export function Section({ title, sub, cls, children, action }: { title: string; sub?: string; cls?: string; children: preact.ComponentChildren; action?: preact.ComponentChildren }) {
  return (
    <section class={"panel " + (cls ?? "")}>
      <div class="phead"><h2>{title}</h2>{sub && <span class="meta">{sub}</span>}<span class="sp" />{action}</div>
      {children}
    </section>
  );
}
const Quiet = ({ children }: { children: preact.ComponentChildren }) => <div class="meta quiet">{children}</div>;

function Ask({ q }: { q: Question }) {
  return (
    <div class="hrow ask">
      <Dotc c="waiting" />
      <div class="ht">
        <div><TLink id={q.thread}>{q.thread_title || q.title || "A question"}</TLink></div>
        <div class="meta">
          {q.approval ? "wants to run" : "asks"}, on {q.node}{q.place ? `, in ${q.place}` : ""}{q.session ? ` (${q.session})` : ""} · {ago(q.asked)}
        </div>
        {q.command ? <pre>{q.command}</pre> : !q.approval && <div class="qtext">{q.title}</div>}
        {!src.answer ? (
          <div class="meta hint">Answer it in the Stitch client or app, the TUI (Ctrl+T), or <Cmd>{`stitch answer ${q.id} y`}</Cmd></div>
        ) : q.approval ? (
          <div class="btns">
            <button class="primary" onClick={() => void src.answer!(q.id, true, false)}>Allow</button>
            <button title={q.grant.length ? "Allow, and from then on: " + q.grant.join(", ") : "Allow, and remember"} onClick={() => void src.answer!(q.id, true, true)}>Always</button>
            <button class="danger" onClick={() => void src.answer!(q.id, false, false)}>Decline</button>
            {q.thread && <button onClick={() => open(q.thread)}>Open thread</button>}
          </div>
        ) : (
          q.thread && <div class="btns"><button class="primary" onClick={() => open(q.thread)}>Answer in thread</button></div>
        )}
      </div>
    </div>
  );
}

function Blocked({ t }: { t: ThreadItem }) {
  return (
    <div class="hrow">
      <Dotc c="waiting" />
      <div class="ht">
        <TLink id={t.id}>{titleOf(t)}</TLink>
        <div class="meta">{outcomeOf(t)}</div>
        <div class="meta">{t.node} · {t.place} · {ago(lastOf(t))}</div>
        <div class="btns">
          {src.open && <button onClick={() => open(t.id)}>Open</button>}
          <button title="Hide it until the thread moves again" onClick={() => void H.dismissThread(t.id)}>Dismiss</button>
        </div>
      </div>
    </div>
  );
}

function scheduleName(x: Schedule) {
  return x.self_wake ? `Wake-up of ${x.thread_title || x.thread || "a thread"}` : x.name;
}

export function Needs() {
  const qs = H.questions.value, bl = H.blocked.value, st = H.stalled.value;
  return (
    <Section title="Needs you" sub={H.needsCount.value ? String(H.needsCount.value) : ""} cls={H.needsCount.value ? "ask" : ""}>
      {qs.map((q) => <Ask key={q.id} q={q} />)}
      {bl.map((t) => <Blocked key={t.id} t={t} />)}
      {st.map((x) => (
        <div class="hrow" key={x.id}><Dotc c="failed" /><div class="ht"><b>{scheduleName(x)}</b><div class="meta">schedule paused by {x.node} after failed firings{x.here?.last_said ? `: ${x.here.last_said}` : ""}</div>
          {src.schedule ? (
            <div class="btns">
              <button class="primary" onClick={() => void H.resumeSchedule(x.id)}>Resume</button>
              <button title="Leave it paused and stop counting it" onClick={() => void H.dismissSchedule(x.id)}>Dismiss</button>
            </div>
          ) : <div class="meta hint">Resume it with <Cmd>{`stitch schedule resume ${x.id}`}</Cmd></div>}
          </div></div>
      ))}
      {!H.needsCount.value && <Quiet>Nothing waits on you.</Quiet>}
    </Section>
  );
}

export function Working() {
  const working = threads.value.filter((t) => t.state === "working");
  const j = H.jobs.value;
  const running = (j?.jobs ?? []).filter((x) => x.running);
  const ended = (j?.jobs ?? []).filter((x) => !x.running);
  return (
    <Section title="What's going on" sub={`${working.length} working · ${running.length} job${running.length === 1 ? "" : "s"}`}>
      {working.map((t) => (
        <div class="hrow" key={t.id}>
          <Dotc c="working" />
          <div class="ht">
            <TLink id={t.id}>{titleOf(t)}</TLink>
            {t.in_flight ? (
              <>
                <pre class="call">{t.in_flight.summary}</pre>
                <div class="running"><i class="spin" />running {t.in_flight.headline}{t.in_flight.since ? ` · ${spoken(t.in_flight.since)}` : ""}</div>
              </>
            ) : (
              <div class="meta">{sentence(t.last_words) || (t.last_step ? "working…" : t.said) || "working…"}</div>
            )}
            <div class="meta">{t.node} · {t.place} · {t.last_step ? "last step" : "sent"} {ago(lastOf(t))}</div>
          </div>
        </div>
      ))}
      {!working.length && <Quiet>No thread is working now.</Quiet>}
      {running.length > 0 && <div class="meta sub">On {j!.node}, in the background:</div>}
      {running.map((x, i) => <Job key={i} j={x} />)}
      {ended.length > 0 && (
        <details><summary>jobs ended in the last hour ({ended.length})</summary>{ended.map((x, i) => <Job key={i} j={x} />)}</details>
      )}
    </Section>
  );
}

export function Job({ j }: { j: LiveJob }) {
  const c: Dot = j.running ? "working" : GOOD.test(j.how ?? "") ? "done" : "failed";
  return (
    <div class="hrow">
      <Dotc c={c} />
      <div class="ht">
        <details>
          <summary>
            <b>{j.job != null ? "job " + j.job : "session " + j.session}</b> {j.thread_title || j.thread} · {j.running ? j.condition : j.how} · {ago(j.started)}
          </summary>
          {j.command ? <pre>{j.command}</pre> : j.file ? <div class="meta">{j.file}</div> : null}
          {j.last_lines && <pre>{j.last_lines}</pre>}
          {canOpen(j.thread) && <TLink id={j.thread}>open thread</TLink>}
        </details>
      </div>
    </div>
  );
}

export function Scheduled() {
  const all = H.schedules.value?.schedules ?? [];
  const active = all.filter((x) => x.state === "active");
  const paused = all.filter((x) => x.state !== "active");
  const one = (x: Schedule) => {
    const next = x.next[0];
    const c = x.firing ? "working" : x.state !== "active" ? "paused" : x.here?.failures ? "failed" : "sched";
    const how = [x.self_wake ? (x.at ? "once" : "when its check finds something") : x.when.replace(/`/g, ""), x.node, x.place].filter(Boolean).join(" · ");
    const facts: string[] = [];
    if (x.check) facts.push(x.check === "code" ? "checks first" : "checks " + x.check);
    if (x.budget?.usd != null) facts.push(`≤ $${x.budget.usd.toFixed(2)} a firing`);
    if (x.daily_cap != null) facts.push(`$${(x.today || 0).toFixed(2)} of $${x.daily_cap.toFixed(2)} today`);
    if (x.read_only) facts.push("read-only");
    if (x.here?.fired) facts.push(`fired ${x.here.fired}×`);
    const last = x.here?.last ? `last ${ago(x.here.last)}${x.here.last_said ? ": " + x.here.last_said : ""}` : "";
    return (
      <div class={"hrow" + (x.state !== "active" ? " paused" : "")} key={x.id}>
        <Dotc c={c} />
        <div class="ht">
          <details>
            <summary><b>{scheduleName(x)}</b><div class="meta">{how}</div>{x.goal && <div class="meta">{sentence(x.goal, 120)}</div>}{last && <div class="meta">{last}</div>}</summary>
            <pre>{x.goal}</pre>
            {x.next.length > 1 && <div class="meta">next: {x.next.map(when).join(", ")}</div>}
            {facts.length > 0 && <div class="meta">{facts.join(" · ")}</div>}
            {x.latest_thread && <div class="meta">its thread: <TLink id={x.latest_thread}>{x.latest_title || x.latest_thread}</TLink></div>}
          </details>
        </div>
        <div class="when">{x.firing ? <b>firing</b> : next ? <><b>{until(next)}</b>{when(next)}</> : x.state}</div>
      </div>
    );
  };
  return (
    <Section title="Scheduled" sub={active.length ? `${active.length} active` : ""}>
      {active.length ? active.map(one) : <Quiet>Nothing is scheduled.</Quiet>}
      {paused.length > 0 && <details><summary>paused ({paused.length})</summary>{paused.map(one)}</details>}
    </Section>
  );
}

const MODES: { m: AutoMode; label: string; says: string }[] = [
  { m: "off", label: "Off", says: "Loose ends only tell you what waits on you." },
  { m: "dry-run", label: "Dry run", says: "Proposes and records what your rules would do, and does nothing." },
  { m: "on", label: "On", says: "Continues or dismisses what your rules cover; you get a push for each." },
];
const RULES_HINT = `For example:
When the agent laid out a plan in stitch or otter and only waits for a go-ahead, tell it to go ahead.
Dismiss research chats nobody touched for 3 days.
Never approve design docs, deploys, purchases or anything with an approval prompt.`;

function AutoPanel() {
  const a = H.autoSetting.value;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [saved, setSaved] = useState(false);
  if (!a) return null;
  const mode = a.mode;
  const pinned = a.here != null && a.here !== a.synced;
  const noRules = !a.rules.trim();
  const edit = () => { setDraft(a.rules); setEditing(true); setSaved(false); };
  const save = async () => {
    if (await H.setAuto({ rules: draft })) { setEditing(false); setSaved(true); }
  };
  const says = MODES.find((x) => x.m === mode)!.says;
  if (!src.looseAuto) {
    // The dashboard shows it only: its endpoint can be called by any local process.
    const label = MODES.find((x) => x.m === mode)!.label;
    return (
      <div class={"auto " + mode}>
        <div class="autohead">
          <span class="autolabel">Auto-handle</span>
          <span class={"modechip " + mode}>{label}</span>
          {pinned && <span class="meta">kept by this node; yours is {a.synced}</span>}
        </div>
        <div class="meta">{says}{mode !== "off" && noRules && <> <b class="warn">No rules yet: it does nothing until you write some.</b></>}</div>
        {!noRules && <details class="rulesshow"><summary>Your rules</summary><pre>{a.rules}</pre></details>}
        <div class="meta hint">Change it in the Stitch client, or with <Cmd>stitch loose-ends --auto off|dry-run|on</Cmd> and <Cmd>--rules edit</Cmd></div>
      </div>
    );
  }
  return (
    <div class={"auto " + mode}>
      <div class="autohead">
        <span class="autolabel">Auto-handle</span>
        <div class="seg" role="radiogroup" aria-label="Auto-handling">
          {MODES.map((x) => (
            <button
              key={x.m} role="radio" aria-checked={mode === x.m} class={mode === x.m ? "on " + x.m : ""}
              disabled={H.autoSaving.value || pinned} title={x.says}
              onClick={() => mode !== x.m && void H.setAuto({ mode: x.m })}
            >{x.label}</button>
          ))}
        </div>
        <span class="sp" />
        {!editing && <button class="link" onClick={edit}>{noRules ? "Write rules" : "Edit rules"}</button>}
      </div>
      <div class="meta">
        {pinned ? `This node's [loose_ends] auto keeps it ${mode}; your setting, synced, is ${a.synced}.` : says}
        {mode !== "off" && noRules && !editing && <> <b class="warn">No rules yet: it does nothing until you write some.</b></>}
        {saved && !editing && <> <span class="ok">Rules saved, synced to every node.</span></>}
      </div>
      {editing ? (
        <div class="rules">
          <textarea
            rows={Math.min(12, Math.max(5, draft.split("\n").length + 1))} value={draft} placeholder={RULES_HINT}
            onInput={(e) => setDraft((e.target as HTMLTextAreaElement).value)}
            onKeyDown={(e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void save(); if (e.key === "Escape") setEditing(false); }}
          />
          <div class="meta">In your words: what loose ends may continue (and what to tell the agent), what to dismiss, and what never to touch. A continue is checked against them by your default model first; approvals and questions are never answered.</div>
          <div class="btns">
            <button class="primary" disabled={H.autoSaving.value || draft === a.rules} onClick={() => void save()}>{H.autoSaving.value ? "Saving…" : "Save rules"}</button>
            <button onClick={() => setEditing(false)}>Cancel</button>
          </div>
        </div>
      ) : (
        !noRules && <details class="rulesshow"><summary>Your rules</summary><pre>{a.rules}</pre></details>
      )}
    </div>
  );
}

const AUTO_SAID: Record<string, { word: string; cls: string }> = {
  "continued": { word: "Auto-continued", cls: "did" },
  "dismissed": { word: "Auto-dismissed", cls: "did" },
  "would-continue": { word: "Would continue", cls: "would" },
  "would-dismiss": { word: "Would dismiss", cls: "would" },
  "declined": { word: "Not continued: the check said no", cls: "held" },
  "held": { word: "Held", cls: "held" },
  "failed": { word: "Auto-handling failed", cls: "bad" },
};

function AutoLine({ a }: { a: LooseAuto }) {
  const said = AUTO_SAID[a.outcome] ?? { word: a.outcome, cls: "held" };
  return (
    <div class={"autoline " + said.cls}>
      <span class="chip">{said.word}{a.at && said.cls === "did" ? " · " + ago(a.at) : ""}</span>
      {a.message && <span class="said">“{a.message}”</span>}
      {a.why && <span class="meta"> {a.why}</span>}
    </div>
  );
}

export function Loose() {
  const L = H.loose.value;
  const items = H.looseItems.value;
  const recent = items.filter((i) => now.value - Date.parse(i.since) < 86400e3);
  const older = items.filter((i) => !recent.includes(i));
  const facts: string[] = [];
  if (L?.last_pass?.at) facts.push(`checked ${ago(L.last_pass.at)}${L.last_pass.model ? " by " + L.last_pass.model.split("/").pop() : ""}`);
  if (L?.unjudged) facts.push(`${L.unjudged} not judged yet`);
  const row = (i: typeof items[number]) => (
    <div class="hrow" key={i.thread + i.since}>
      <Dotc c={i.verdict === "waiting-on-you" ? "waiting" : i.verdict === "failed" ? "failed" : "done"} />
      <div class="ht">
        <TLink id={i.thread}>{i.title}</TLink>{i.unread && <span class="chip unread">unread</span>}
        <div class="meta">{i.line ?? (i.verdict ? i.verdict : "not judged yet")}</div>
        {i.auto && <AutoLine a={i.auto} />}
        <div class="meta">{i.node} · {i.place} · {ago(i.since)}</div>
      </div>
      <button title="Hide it until it moves again" onClick={() => void H.dismissLoose(i)}>Dismiss</button>
    </div>
  );
  return (
    <Section
      title="Loose ends" sub={facts.join(" · ")} cls={items.some((i) => i.verdict === "waiting-on-you") ? "ask" : ""}
      action={L && <button disabled={H.checking.value} onClick={() => void H.checkLoose()}>{H.checking.value ? "Checking…" : "Check loose ends"}</button>}
    >
      {L && <AutoPanel />}
      {!L && <Quiet>This node does not tell loose ends.</Quiet>}
      {L && (recent.length ? recent.map(row) : <Quiet>{older.length ? "Nothing from the last day." : "Nothing waits on you."}</Quiet>)}
      {older.length > 0 && <details><summary>Older ({older.length})</summary>{older.map(row)}</details>}
    </Section>
  );
}

interface Ev { at: string; dot: Dot; what: string; how: string }
const DAYS = 3;

export function Timeline() {
  const [all, setAll] = useState(false);
  const ev: Ev[] = [];
  for (const t of threads.value) {
    const what = titleOf(t);
    if (!t.started && !t.last_step) continue;
    if (t.state === "working") ev.push({ at: t.started ?? lastOf(t), dot: "working", what, how: `started on ${t.node}, working` });
    else if (t.state === "blocked") ev.push({ at: lastOf(t), dot: "waiting", what, how: `waits for you, on ${t.node}` });
    else {
      const how = t.state === "cancelled" ? "cancelled" : "reported " + (t.outcome || "done").replace(/-/g, " ");
      ev.push({ at: lastOf(t), dot: dotOf(t), what, how: `${how} on ${t.node}` });
    }
  }
  for (const x of H.schedules.value?.schedules ?? []) {
    const last = x.here?.last;
    if (!last || now.value - Date.parse(last) > DAYS * 86400e3) continue;
    const said = x.here!.last_said || "fired";
    ev.push({ at: last, dot: /fail|over budget|not read/i.test(said) ? "failed" : "done", what: scheduleName(x), how: `${said}, on ${x.node}` });
  }
  ev.sort((a, b) => Date.parse(b.at) - Date.parse(a.at));
  const shown = all ? ev : ev.slice(0, 30);
  let day = "";
  return (
    <Section title="Timeline" sub="threads and schedules, latest first">
      {shown.map((e, i) => {
        const d = dayOf(e.at);
        const head = d !== day ? <div class="day">{d}</div> : null;
        day = d;
        return <>{head}<div class="ev" key={i}><time>{hhmm(e.at)}</time><Dotc c={e.dot} /><span>{e.what} <span class="meta">· {e.how}</span></span></div></>;
      })}
      {!ev.length && <Quiet>Nothing moved yet.</Quiet>}
      {ev.length > shown.length && <button class="link" onClick={() => setAll(true)}>Show all {ev.length}</button>}
    </Section>
  );
}

export function Home({ lead }: { lead?: preact.ComponentChildren } = {}) {
  useClock();
  const ts = threads.value;
  return (
    <main class="home">
      <header class={lead ? "withswitch" : undefined}><h1>Stitch</h1>{lead}<span class="meta">{ts.filter((t) => t.state === "working").length} working · {H.needsCount.value} need you</span></header>
      <div class="homebody">
        {H.liveError.value && <div class="err" onClick={() => (H.liveError.value = null)}>{H.liveError.value}</div>}
        <Needs />
        <Working />
        <Scheduled />
        <Loose />
        <Timeline />
      </div>
    </main>
  );
}
