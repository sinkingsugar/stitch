// Where the home view's data comes from and what it may do (design §2.10,
// §2.24). Two hosts render the same views: the web client (the WebSocket, the
// user's full reach) and the built-in dashboard (`stitch.live`, read-only but
// for loose ends' check and dismiss). The views read these signals and call
// `src`; a capability a host lacks is absent, and the views say how to do it
// elsewhere instead of showing a button.
import { signal } from "@preact/signals";
import type { ThreadItem } from "../model";
import type { AutoMode, AutoSetting } from "./payload";

/** The thread list (every node's): the client's `Request::Threads`, the dashboard's `live.threads`. */
export const threads = signal<ThreadItem[]>([]);
/** The home view is shown (no thread open). Always true in the dashboard. */
export const homeView = signal(true);

export interface Source {
  /** `Request::Live` / `stitch.live.<what>()`: attention, jobs, schedules, loose-ends. */
  live<T>(what: string): Promise<T>;
  /** A loose-ends check pass, or a dismissal (the dashboard's only writes). */
  looseEnds(op: { check?: boolean; dismiss?: string }): Promise<unknown>;
  /** Opens a thread: the client selects it; the dashboard has a link (`href`) on the phone, or nothing. */
  open?(id: string): void;
  /** A link that opens a thread (the phone's `stitch://thread?id=`). */
  href?(id: string): string | null;
  /** Answers an approval (client only). */
  answer?(question: string, allow: boolean, always: boolean): Promise<unknown>;
  /** Sets loose ends' auto-handling (client only: the dashboard's endpoint is reachable by any local process). */
  looseAuto?(set: { mode?: AutoMode; rules?: string }): Promise<AutoSetting>;
  /** `Schedule` pause / resume as the user (client only). */
  schedule?(input: { action: string; schedule: string }): Promise<unknown>;
}

export let src: Source = {
  live: () => Promise.reject(new Error("not connected")),
  looseEnds: () => Promise.reject(new Error("not connected")),
};
export function setSource(s: Source) { src = s; }
/** Whether a thread can be opened from here (a click, a link). */
export const canOpen = (id: string | null | undefined): id is string => !!id && (!!src.open || !!src.href?.(id));
