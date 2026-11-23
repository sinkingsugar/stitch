// The payloads of `Request::Threads`, `Request::Transcript`, `Request::Artefacts`,
// `Request::Touched` and `Request::Digest`. Their replies
// are `Reply::Lines([json])`, built with `serde_json::json!` on the node
// (`apps::live::threads`, `apps::threads::transcript`, `artefacts.rs`), not from Rust structs,
// so ts-rs cannot derive them: these two are written by hand to match, here and
// nowhere else. Change them with those functions.
import type { Compaction } from "./api/Compaction";

/** One thread of the list. Instants are RFC 3339 strings. */
export interface ThreadRow {
  id: string;
  title: string;
  goal: string;
  node: string;
  started_on: string | null;
  place: string;
  parent: string | null;
  parent_title: string | null;
  detached: boolean;
  read_only: boolean;
  /** Pinned by the user (`Request::Pin`), on any thread of its compaction
   * chain: first in lists. */
  pinned?: boolean;
  /** Activity after the conversation's read cursor (`Request::Read`); never read counts as unread. */
  unread?: boolean;
  /** The threads this one was compacted from, oldest first (design §2.15):
   * a compaction chain is one row, its newest thread; a client open on one
   * of these is open on this one. */
  earlier?: string[];
  /** The conversation's digest (design §2.25): its headline, how many open
   * questions and asks of the user it holds, when it was written, and whether
   * it covers everything the thread has done (`fresh`). Absent when there is none. */
  digest?: RowDigest;
  app: unknown;
  state: "working" | "blocked" | "idle" | "ended" | "cancelled";
  outcome: string | null;
  said: string;
  /** An approval waiting: `Request::Answer { question: id, .. }` answers it. */
  question: { id: string; command: string } | null;
  started: string | null;
  last_step: string | null;
  model_calls: number;
  cost: number | null;
  last_words: string | null;
  in_flight: { name: string; headline: string; summary: string; since: string | null } | null;
  /** A room (design §2.27, `apps::live`): absent on other threads. */
  room?: boolean;
  /** A room's participants, as `RoomInfo` lists them. */
  participants?: RoomParticipant[];
  room_mode?: "sequential" | "parallel";
}

/** One of a room's participants (`rooms::participants`): its `@slug`, how
 * it is shown, its model and its own thread (the newest of its chain). */
export interface RoomParticipant {
  slug: string;
  name: string;
  model: string | null;
  thread: string;
  muted: boolean;
}

/** `Reply::Lines([json])` of `RoomCreate`, `RoomInfo` and `RoomUpdate`
 * (`rooms::info`), and a room's `Transcript.room`. */
export interface RoomInfo {
  room: string;
  title: string | null;
  /** The thread "Pull in model…" made it from. */
  promoted_from: string | null;
  budget_usd: number | null;
  mode: "sequential" | "parallel";
  hop_cap: number;
  participants: RoomParticipant[];
  /** A round runs on it now. */
  running: boolean;
}

/** Who said a room's reply (`from = participant:<slug>:<thread>`). */
export interface Speaker {
  slug: string;
  name: string;
  thread: string;
  model: string | null;
}

/** On a participant's own thread: the message the room relayed to it, up to
 * which of the room's messages. */
export interface RoomTurnRef {
  room: string;
  upto: string;
  slug: string;
}

/** A thread row's digest (`Request::Digest` has the whole one). */
export interface RowDigest {
  headline: string;
  asks: number;
  questions: number;
  /** When it was written (RFC 3339). */
  at: string;
  /** False when the thread moved after what the digest read. */
  fresh: boolean;
}

/** `Reply::Lines([JSON.stringify(ThreadList)])` of `Request::Threads`. */
export interface ThreadList {
  node: string;
  at: string;
  threads: ThreadRow[];
  more: number;
  /** Every place of the listed threads (not only the window's) with how many
   * it has, as a row's `place` names it: the user's own list (`every`) only. */
  places?: PlaceCount[];
}

export interface PlaceCount {
  place: string;
  count: number;
  /** Present, `false`, only when the place's settings say `memory = false` (design §2.9). */
  memory?: false;
}

export interface ImageRef {
  hash: string;
  type: string;
  name: string | null;
}

/** Any other file a message carries (not an image): downloaded from `/_stitch/blob/<hash>`. */
export interface FileRef {
  hash: string;
  type: string;
  name: string | null;
  bytes: number;
}

/** A preset answer: the button's `label`, the message pressing it sends. */
export interface SuggestedReply {
  label: string;
  text: string;
}

/** An action button (design §2.30, step 2): its label, the call it runs as
 * one line, and whether its press also wakes the agent. */
export interface ActionButton {
  label: string;
  tool: string;
  call: string;
  wake: boolean;
}

/** A transcript entry; `n` numbers the lineage's events. */
export type TranscriptEntry =
  | { n: number; id?: string; /** The message's durable name, `<thread>#<id>` (also for an earlier thread's of a compaction chain; none for what a fork inherits): what a digest's items cite. */ mid?: string | null; inherited?: boolean; kind: "user" | "assistant"; from: string | null; at: string; text: string; images: ImageRef[]; files?: FileRef[]; /** Said in a voice call (design §2.26): a transcription, marked. */ spoken?: boolean; /** Threads and efforts attached to it (design §2.27): chips; their briefing is the model's, not shown. */ scopes?: ScopeChip[]; /** A room's reply: who said it (design §2.27). */ speaker?: Speaker; /** A participant's relayed room message. */ room_turn?: RoomTurnRef; /** Sent from here, not yet in the node's transcript. */ pending?: boolean; /** Where it stands meanwhile (transcript/outbox.ts). */ outgoing?: import("./transcript/outbox").Outgoing; /** Why it was not sent. */ failed?: string }
  | {
      n: number;
      inherited?: boolean;
      kind: "tool";
      at: string;
      name: string;
      call: string;
      result: { ok: boolean; output: string } | null;
      /** Images the tool showed (view_file, screenshots, generate_image). */
      images?: ImageRef[];
      /** An app `show_app` took: `AppOpen { what: manifest }` gives its address. */
      app?: { manifest: string; name: string | null } | null;
      /** `edit_file` / `write_file`: a unified diff (from `@@` on). */
      diff?: { path: string; text: string } | null;
      /** `suggest_replies`: preset answers (design §2.30), buttons at the
       * turn's end (transcript/suggested.ts says when they are live). */
      replies?: SuggestedReply[];
      /** `offer_actions`, accepted: its buttons (design §2.30, step 2) and
       * the set's ID a press names (transcript/suggested.ts). */
      actions?: ActionButton[];
      set?: string;
    }
  /** A room's note (design §2.27: a participant's question settled), kept in its transcript. */
  | { n: number; inherited?: boolean; kind: "note"; at: string; text: string }
  /** An action button pressed: its line ("▶ label: ok"), the call's output. */
  | { n: number; inherited?: boolean; kind: "press"; at: string; text: string; set: string; item: number; label: string; tool: string; ok: boolean; output: string; wake: boolean }
  /** A turn whose model call failed (a sign-in that expired, an account out
   * of usage): why, as the node says it. */
  | { n: number; inherited?: boolean; kind: "failed"; at: string; text: string }
  /** Where the conversation was compacted (design §2.15): the thread `from`
   * goes on in `to`, from `summary` and its last `kept` messages. `said` is
   * the divider's line ("compacted here (162k of 200k window): summary +
   * last 10 messages"); `n` is the event it comes before. */
  | ({ n: number; kind: "compacted"; at: string; said: string } & Compaction);

/** `Reply::Lines([JSON.stringify(Transcript)])` of `Request::Transcript`: at
 * most 500 entries; `next` is the `after` to read the rest from. */
export interface Transcript {
  thread: string;
  entries: TranscriptEntry[];
  next: number;
  /** The thread a fork continues (its first entries are what it inherits). */
  forked_from?: { thread: string; title: string | null } | null;
  /** The newest thread of its compaction chain: where the conversation goes
   * on (`thread` itself unless it was compacted). */
  current?: string;
  /** A room: what `RoomInfo` says of it. */
  room?: RoomInfo;
  /** A window (`tail`): the `n` of its first entry, the `before` that reads what precedes it. */
  start?: number;
  /** A window (`tail`): entries precede it. */
  earlier?: boolean;
}

export type ForkedFrom = { thread: string; title: string | null } | null;

export type ArtefactKind = "path" | "url" | "commit" | "thread";

/** What a conversation did to one artefact (`Request::Artefacts`), over its
 * compaction chain. Messages are durable names, `<thread>#<id>`. */
export interface ArtefactItem {
  value: string;
  verb: "read" | "wrote" | "spawned" | "told";
  first_at: string;
  last_at: string;
  /** How many tool calls touched it. */
  n: number;
  /** The message of the latest touch, and of the first. */
  msg: string | null;
  first_msg: string | null;
  /** The chain's thread that touched it last. */
  thread: string;
  /** A commit's subject line. */
  subject: string | null;
}

/** `Reply::Lines([JSON.stringify(Artefacts)])` of `Request::Artefacts`: each
 * kind's items, the latest first. */
export interface Artefacts {
  /** The newest thread of the chain. */
  thread: string;
  chain: string[];
  kinds: Record<ArtefactKind, ArtefactItem[]>;
}

export type DigestKind = "goal" | "outcome" | "decision" | "finding" | "change" | "question" | "ask";

/** One item of a digest, anchored to the message it comes from: open
 * `thread` at `msg` (`at` is `<thread>#<msg>`). */
export interface DigestItem {
  kind: DigestKind;
  text: string;
  at: string;
  thread: string;
  msg: string;
}

/** `Reply::Lines([JSON.stringify(Digest)])` of `Request::Digest`: a
 * conversation's newest thread's digest, else the latest of its earlier
 * threads'; `digest` is null when none was written. `update` is there when
 * the request asked for one. */
export interface Digest {
  /** The conversation's newest thread. */
  thread: string;
  digest: {
    record: string;
    /** The thread it was written for (an earlier one of the chain, maybe). */
    thread: string;
    headline: string;
    /** Most important first. */
    items: DigestItem[];
    model: string;
    /** What it cost over its updates. */
    usd: number;
    /** The last message it covers, `<thread>#<msg>`. */
    upto: string | null;
    at: string;
    fresh: boolean;
  } | null;
  update?: {
    outcome: "written" | "nothing new" | "capped" | "busy" | "failed";
    why: string | null;
    usd: number;
    model: string;
    windows: number;
    more: boolean;
    dropped: number;
    day_usd: number;
    cap_usd: number;
  };
}

/** One conversation's touch of an artefact (`Request::Touched`), per verb. */
export interface TouchedConversation {
  /** The conversation's newest thread. */
  thread: string;
  title: string | null;
  /** The place's label, as a thread row's `place`. */
  place: string;
  verb: ArtefactItem["verb"];
  first_at: string;
  last_at: string;
  n: number;
  msg: string | null;
  first_msg: string | null;
  subject: string | null;
}

/** `Reply::Lines([JSON.stringify(Touched)])` of `Request::Touched`, in the
 * time of the first touch. */
export interface Touched {
  kind: ArtefactKind;
  /** As the index keeps it (a URL without its fragment). */
  value: string;
  conversations: TouchedConversation[];
}

/** A thread or effort attached to a message (`Part::Scope`, design §2.27). */
export type ScopeChip = { kind: "effort" | "thread"; id: string; title?: string | null; threads: number };
