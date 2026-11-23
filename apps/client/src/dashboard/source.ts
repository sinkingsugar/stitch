// The built-in dashboard's source (design §2.10): the node's live state through
// the apps bridge (`stitch.live`, `/_stitch/live/*`), kept current by its change
// events. Read-only but for loose ends' check and dismiss, the only POSTs the
// node takes from the dashboard: no approvals answered, no schedules moved, no
// auto-handling set here (any local process could imitate this page).
import { signal } from "@preact/signals";
import { setSource, threads } from "../home/source";
import { refreshLive } from "../home/state";
import { coalesced } from "../coalesce";
import type { ThreadList } from "../payload";

interface LiveError extends Error { consent?: "ask" | "denied"; allow?: string }
interface LiveBridge {
  threads(o: { all?: boolean; since?: Date; limit?: number }): Promise<ThreadList>;
  attention(): Promise<unknown>;
  jobs(): Promise<unknown>;
  schedules?(): Promise<unknown>;
  looseEnds?(): Promise<unknown>;
  checkLooseEnds(): Promise<unknown>;
  dismissLooseEnd(thread: string): Promise<unknown>;
  on(kind: string, f: () => void): void;
}
declare global {
  interface Window { stitch?: { live: LiveBridge }; webkit?: { messageHandlers?: unknown } }
}

/** Threads of the last three days, and those working or waiting. */
export const DAYS = 3;
/** Whether the page shows live data; when it read last. */
export const live = signal<{ ok: boolean; at: Date | null }>({ ok: false, at: null });
/** Why the threads could not be read: consent asked or denied, or an error. */
export const problem = signal<{ text: string; consent: boolean } | null>(null);

/** The phone's web view opens a thread through the app's own link. */
const NATIVE = !!window.webkit?.messageHandlers;

export function initDashboard() {
  const L = window.stitch?.live;
  if (!L) {
    problem.value = { text: "This page reads the node's live state through the apps bridge: open it with `stitch dashboard`.", consent: false };
    return;
  }
  setSource({
    live: <T,>(what: string) => {
      const f = what === "attention" ? L.attention : what === "jobs" ? L.jobs : what === "schedules" ? L.schedules : what === "loose-ends" ? L.looseEnds : undefined;
      // A node older than schedules or loose ends in live state answers 404: the rest shows.
      return f ? (f.call(L) as Promise<T>) : Promise.reject(new Error("this node does not say " + what));
    },
    looseEnds: (op) => (op.check ? L.checkLooseEnds() : L.dismissLooseEnd(op.dismiss!)),
    href: NATIVE ? (id) => `stitch://thread?id=${encodeURIComponent(id)}` : undefined,
  });
  let timer: ReturnType<typeof setTimeout> | undefined;
  L.on("*", () => { clearTimeout(timer); timer = setTimeout(() => void load(), 300); });
  // Times move on; a running thread's step is not an event.
  setInterval(() => void load(), 60000);
  void load();
}

const loadThreads = coalesced(async () => {
  const L = window.stitch!.live;
  try {
    const r = await L.threads({ all: true, since: new Date(Date.now() - DAYS * 86400e3), limit: 300 });
    threads.value = r.threads;
    problem.value = null;
    live.value = { ok: true, at: new Date() };
  } catch (e) {
    const err = e as LiveError;
    problem.value = { text: err.consent ? err.message : "Threads: " + (err.message || String(e)), consent: !!err.consent };
    live.value = { ok: false, at: live.value.at };
  }
});

export async function load() {
  await Promise.all([loadThreads(), problem.value?.consent ? null : refreshLive()]);
}
