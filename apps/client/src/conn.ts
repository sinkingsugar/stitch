// The connection layer. Everything the views know about the node is the `Api`
// interface; everything the wire looks like is in the ENVELOPE section below,
// so a different envelope is a change to this file only.
import type { Request } from "./api/Request";
import type { Reply } from "./api/Reply";
import type { Followed, LiveEvent, ThreadItem, Entry, TurnStatus } from "./model";
import type { ForkedFrom, PlaceCount, RoomInfo, ThreadList, Transcript } from "./payload";
import type { AutoMode, AutoSetting } from "./home/payload";
import { CLIENT_VERSION } from "./version";
import type { ListPush } from "./deltas";
import type { Runner } from "./runs/state";

/** What part of a transcript to read (`Api.transcript`). */
export type TranscriptAt = { after?: number; before?: number; tail?: number };
export type TranscriptRead = { entries: Entry[]; forked_from: ForkedFrom; current?: string; room?: RoomInfo | null; start: number; earlier: boolean };

export interface Api {
  /** A protocol request and its (one) reply (`Reply::Error` rejects; so does
   * no reply within `timeoutMs`, when given). */
  call(req: Request, timeoutMs?: number): Promise<Reply>;
  /** A request that streams replies (`Chat`): each one to `onReply`, until
   * `ChatDone` or an error (which rejects). */
  stream(req: Request, onReply: (r: Reply) => void): Promise<void>;
  /** Stream a thread's turn events; returns the unfollow function. */
  follow(thread: string, onEvent: (e: Followed) => void): () => void;
  /** The thread list (every node's, through this one): `Request::Threads`. */
  listThreads(): Promise<{ threads: ThreadItem[]; places: PlaceCount[] }>;
  /** The whole forest (`Threads { forest }`: every conversation and spawned thread, lean rows, up to 5000): the map's data. */
  listForest(): Promise<ThreadItem[]>;
  /** One place's threads (`Threads { place }`, the limit after the filter):
   * what the window leaves out of a section. */
  placeThreads(place: string): Promise<ThreadItem[]>;
  /** One thread's row, wherever it falls in the list (`Threads { thread }`):
   * a thread opened from outside the list's window. Null: none such. */
  thread(id: string): Promise<ThreadItem | null>;
  /** A thread's transcript: `Request::Transcript`. `tail`: the last that
   * many entries before `before` (one window, `start` its first entry's `n`,
   * `earlier` whether more precede it); else from `after` on, paged to the end. */
  transcript(thread: string, at?: TranscriptAt): Promise<TranscriptRead>;
  /** `Request::Live`: `attention`, `jobs`, `schedules` or `loose-ends`. */
  live<T>(what: string, query?: string): Promise<T>;
  /** `Request::LooseEnds`: a check pass, or a dismissal. */
  looseEnds(op: { check?: boolean; dismiss?: string }): Promise<void>;
  /** `Request::LooseEndsAuto`: auto-handling, set when given (mode, rules). */
  looseAuto(set?: { mode?: AutoMode; rules?: string }): Promise<AutoSetting>;
  /** `Request::Schedule` as the user (`pause`, `resume`, …): the lines it says. */
  schedule(input: { action: string; schedule: string }): Promise<string[]>;
  /** `Request::RunsOn`: the node the user had the thread run on, if any. */
  runsOn(thread: string): Promise<string | null>;
  /** `Request::Runners`: the other nodes, `runs`: yes / no / not reached (null); `otherBuild`: reached, but on a build whose answers this node cannot read. */
  runners(): Promise<Runner[]>;
  /** `Request::RunOn`: continue the thread on `node`; null brings it back here. */
  runOn(thread: string, node: string | null): Promise<string>;
  /** Called with each connection state change. */
  onState(f: (s: ConnState) => void): void;
  /** Called when the node says threads, jobs or approvals changed (re-list). */
  onThreadsChanged(f: () => void): void;
  /** Deltas (design §2.24): absent on a source without them (the mock), which
   * re-lists on `onThreadsChanged`. */
  watch?(): void;
  watchCalls?(threads: string[]): void;
  onList?(f: (p: ListPush) => void): void;
  onLive?(f: (what: string, value: unknown) => void): void;
  onCall?(f: (thread: string, inFlight: ThreadItem["in_flight"]) => void): void;
}
/** `mismatch:<node's version>`: this page's client is of another build than the node, and a reload did not fix it. */
export type ConnState = "connecting" | "open" | "restarting" | `mismatch:${string}`;

// ---------------------------------------------------------------- ENVELOPE
// JSON text frames on /_stitch/ws (apps/ws.rs).
//   client -> node  { id, request: Request }
//                   { follow: { thread, after: null } } / { unfollow: { thread } }
//   node -> client  { id, reply: Reply }      every reply of request `id`
//                   { id, error: string }
//                   { event: { thread, event: LiveEvent } }  a followed turn's event
//                   { event: { thread, gap: true } }         re-read the transcript
//                   { event: { thread, turn: { running, queued } } }  where its turns stand
//                   { changed: true }                        re-list the threads
// Unit variants of Request/Reply are strings, struct variants { "Name": {..} }.
type Out =
  | { id: number; request: Request }
  | { follow: { thread: string; after: number | null } }
  | { unfollow: { thread: string } }
  | { watch: { limit: number } }
  | { calls: { threads: string[] } };
type In = {
  id?: number;
  reply?: Reply;
  error?: string;
  event?: { thread: string; event?: LiveEvent; gap?: true; turn?: TurnStatus };
  changed?: boolean;
  list?: ListPush;
  live?: { what: string; value: unknown };
  call?: { thread: string; in_flight: ThreadItem["in_flight"] };
};
const endpoint = () => `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/_stitch/ws`;
// ------------------------------------------------------------ end ENVELOPE

type Frame = { reply: Reply } | { error: string };
const PAGE = 500;
/** The list's window: the latest threads. Older ones are read one by one
 * when opened (`thread`). */
export const LIST_LIMIT = 300;
/** The node's most for one list (`THREADS_MAX`). */
const PLACE_LIMIT = 500;
const FOREST_LIMIT = 5000;

export class WsConn implements Api {
  private ws: WebSocket | null = null;
  private next = 1;
  private pending = new Map<number, (f: Frame) => void>();
  private followers = new Map<string, Set<(e: Followed) => void>>();
  private stateFns: ((s: ConnState) => void)[] = [];
  private changedFns: (() => void)[] = [];
  private listFns: ((p: ListPush) => void)[] = [];
  private liveFns: ((what: string, value: unknown) => void)[] = [];
  private callFns: ((thread: string, inFlight: ThreadItem["in_flight"]) => void)[] = [];
  private calls: string[] = [];
  private backoff = 250;
  private everOpen = false;
  private stopped = false;

  constructor() {
    this.connect();
  }

  private setState(s: ConnState) {
    console.info("stitch: connection", s);
    this.stateFns.forEach((f) => f(s));
  }

  private connect() {
    this.setState(this.everOpen ? "restarting" : "connecting");
    const ws = new WebSocket(endpoint());
    this.ws = ws;
    ws.onopen = async () => {
      console.info("stitch: socket open, saying Hello");
      try {
        const r = await this.one({ Hello: { version: CLIENT_VERSION } });
        if (typeof r === "object" && "Hello" in r && r.Hello.version !== CLIENT_VERSION) {
          // The node restarted into another build: reload once per node version. Still
          // mismatched after that (the page is cached, or served from another build), so
          // stop instead of looping.
          const node = r.Hello.version;
          const key = "stitch-reloaded-for";
          let tried: string | null = null;
          try { tried = sessionStorage.getItem(key); } catch { /* no storage: never reload */ tried = node; }
          if (tried === node) {
            console.warn("stitch: client", CLIENT_VERSION, "does not match node", node);
            this.stopped = true;
            this.setState(`mismatch:${node}`);
            this.ws?.close();
            return;
          }
          sessionStorage.setItem(key, node);
          location.reload();
          return;
        }
      } catch {
        return; // the socket closes after a refused Hello; onclose retries
      }
      console.info("stitch: connected, Hello passed", CLIENT_VERSION);
      this.everOpen = true;
      this.backoff = 250;
      this.setState("open");
      for (const thread of this.followers.keys()) this.send({ follow: { thread, after: null } });
      // Deltas when someone listens to them (the list comes whole first), else `changed`.
      if (this.listFns.length) {
        this.watch();
        this.send({ calls: { threads: this.calls } });
      } else this.changedFns.forEach((f) => f());
    };
    ws.onmessage = (m) => {
      const msg = JSON.parse(String(m.data)) as In;
      if (msg.id !== undefined && msg.id !== null) {
        const f = msg.error !== undefined ? { error: msg.error } : msg.reply !== undefined ? { reply: msg.reply } : null;
        if (f) this.pending.get(msg.id)?.(f);
      } else if (msg.event) {
        const e: Followed = msg.event.gap ? { gap: true } : msg.event.turn ? { turn: msg.event.turn } : { event: msg.event.event! };
        this.followers.get(msg.event.thread)?.forEach((f) => f(e));
      } else if (msg.list) this.listFns.forEach((f) => f(msg.list!));
      else if (msg.live) this.liveFns.forEach((f) => f(msg.live!.what, msg.live!.value));
      else if (msg.call) this.callFns.forEach((f) => f(msg.call!.thread, msg.call!.in_flight));
      else if (msg.changed) this.changedFns.forEach((f) => f());
      else if (msg.error) console.warn("stitch: node said", msg.error);
    };
    ws.onclose = () => {
      this.pending.forEach((f) => f({ error: "connection closed" }));
      this.pending.clear();
      if (this.stopped) return;
      this.setState("restarting");
      this.everOpen = true;
      setTimeout(() => this.connect(), this.backoff);
      this.backoff = Math.min(this.backoff * 2, 5000);
    };
  }

  private send(o: Out): boolean {
    if (this.ws?.readyState !== WebSocket.OPEN) return false;
    this.ws.send(JSON.stringify(o));
    return true;
  }

  /** Sends a request; `onFrame` hears each frame of its id and returns true
   * when that was the last. */
  private run(request: Request, onFrame: (f: Frame) => boolean): number {
    const id = this.next++;
    this.pending.set(id, (f) => {
      if (onFrame(f)) this.pending.delete(id);
    });
    if (!this.send({ id, request })) {
      this.pending.delete(id);
      onFrame({ error: "not connected" });
    }
    return id;
  }

  private one(request: Request, timeoutMs?: number): Promise<Reply> {
    return new Promise((ok, err) => {
      let timer: ReturnType<typeof setTimeout> | undefined;
      const id = this.run(request, (f) => {
        clearTimeout(timer);
        if ("error" in f) err(new Error(f.error));
        else if (typeof f.reply === "object" && "Error" in f.reply) err(new Error(f.reply.Error));
        else ok(f.reply);
        return true;
      });
      if (timeoutMs && this.pending.has(id)) {
        timer = setTimeout(() => {
          // A late answer finds no one waiting.
          this.pending.delete(id);
          err(new Error(`no answer from the node in ${Math.round(timeoutMs / 1000)} s`));
        }, timeoutMs);
      }
    });
  }

  call(request: Request, timeoutMs?: number) {
    return this.one(request, timeoutMs);
  }
  stream(request: Request, onReply: (r: Reply) => void): Promise<void> {
    return new Promise((ok, err) => {
      this.run(request, (f) => {
        if ("error" in f) return err(new Error(f.error)), true;
        const r = f.reply;
        if (typeof r === "object" && "Error" in r) return err(new Error(r.Error)), true;
        onReply(r);
        if (typeof r === "object" && "ChatDone" in r) return ok(), true;
        return false;
      });
    });
  }
  follow(thread: string, onEvent: (e: Followed) => void) {
    let set = this.followers.get(thread);
    if (!set) this.followers.set(thread, (set = new Set()));
    if (set.size === 0 && this.ws?.readyState === WebSocket.OPEN) this.send({ follow: { thread, after: null } });
    set.add(onEvent);
    return () => {
      set.delete(onEvent);
      if (set.size === 0) {
        this.followers.delete(thread);
        if (this.ws?.readyState === WebSocket.OPEN) this.send({ unfollow: { thread } });
      }
    };
  }
  /** `Reply::Lines([json])`: the one line, parsed. */
  private async lines<T>(request: Request): Promise<T> {
    const r = await this.one(request);
    if (typeof r !== "object" || !("Lines" in r) || r.Lines.length === 0) throw new Error("unexpected reply");
    return JSON.parse(r.Lines[0]!) as T;
  }
  async listThreads() {
    const r = await this.lines<ThreadList>({ Threads: { limit: LIST_LIMIT } });
    return { threads: r.threads, places: r.places ?? [] };
  }
  async listForest() {
    return (await this.lines<ThreadList>({ Threads: { limit: FOREST_LIMIT, forest: true } })).threads;
  }
  async placeThreads(place: string) {
    return (await this.lines<ThreadList>({ Threads: { limit: PLACE_LIMIT, place } })).threads;
  }
  async thread(id: string) {
    return (await this.lines<ThreadList>({ Threads: { thread: id } })).threads[0] ?? null;
  }
  live<T>(what: string, query?: string) {
    return this.lines<T>({ Live: { what, query: query ?? null } });
  }
  async looseEnds(op: { check?: boolean; dismiss?: string }) {
    await this.one({ LooseEnds: { check: !!op.check, dismiss: op.dismiss ?? null } });
  }
  looseAuto(set?: { mode?: AutoMode; rules?: string }) {
    return this.lines<AutoSetting>({ LooseEndsAuto: { mode: set?.mode ?? null, rules: set?.rules ?? null } });
  }
  private async said(request: Request): Promise<string[]> {
    const r = await this.one(request);
    return typeof r === "object" && "Lines" in r ? r.Lines : [];
  }
  schedule(input: { action: string; schedule: string }) {
    return this.said({ Schedule: { input: JSON.stringify(input), cwd: null } });
  }
  async runsOn(thread: string) {
    return (await this.said({ RunsOn: { thread } }))[0] || null;
  }
  async runners() {
    return (await this.said("Runners")).map((l) => {
      const [name = "", runs = ""] = l.split("\t");
      return { name, runs: runs === "yes" ? true : runs === "no" ? false : null, otherBuild: runs === "other-build" };
    });
  }
  async runOn(thread: string, node: string | null) {
    return (await this.said({ RunOn: { thread, node } })).join("\n");
  }
  async transcript(thread: string, at: TranscriptAt = {}): Promise<TranscriptRead> {
    if (at.tail) {
      const t: Transcript = await this.lines<Transcript>({ Transcript: { thread, after: null, before: at.before ?? null, tail: at.tail } });
      return { entries: t.entries, forked_from: t.forked_from ?? null, current: t.current, room: t.room ?? null, start: t.start ?? 0, earlier: !!t.earlier };
    }
    const all: Entry[] = [];
    let forked_from: ForkedFrom = null;
    let current: string | undefined;
    let room: RoomInfo | null = null;
    let after: number | null = at.after ?? null;
    for (;;) {
      const t: Transcript = await this.lines<Transcript>({ Transcript: { thread, after } });
      all.push(...t.entries);
      forked_from = t.forked_from ?? null;
      current = t.current;
      room = t.room ?? null;
      if (t.entries.length < PAGE || t.next === after) return { entries: all, forked_from, current, room, start: at.after ?? 0, earlier: !!at.after };
      after = t.next;
    }
  }
  onState(f: (s: ConnState) => void) {
    this.stateFns.push(f);
  }
  onThreadsChanged(f: () => void) {
    this.changedFns.push(f);
  }
  /** Asks for the list whole, then its deltas (again: a resync). */
  watch() {
    this.send({ watch: { limit: LIST_LIMIT } });
  }
  /** The threads whose running calls this client hears (`call`). */
  watchCalls(threads: string[]) {
    const next = [...threads].sort();
    if (next.join() === this.calls.join()) return;
    this.calls = next;
    this.send({ calls: { threads: next } });
  }
  onList(f: (p: ListPush) => void) {
    this.listFns.push(f);
  }
  onLive(f: (what: string, value: unknown) => void) {
    this.liveFns.push(f);
  }
  onCall(f: (thread: string, inFlight: ThreadItem["in_flight"]) => void) {
    this.callFns.push(f);
  }
}
