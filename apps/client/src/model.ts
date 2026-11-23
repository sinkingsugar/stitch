// What the views show. The protocol types are generated (./api); the payloads
// of the thread list and the transcript are in ./payload.
import type { LiveEvent } from "./api/LiveEvent";
import type { Speaker, ThreadRow, TranscriptEntry } from "./payload";
export type { LiveEvent };

export type ThreadItem = ThreadRow;

/** What a followed thread streams: a turn's event, "some of it was not
 * kept: read the transcript again", or where its turns stand. */
export type Followed = { event: LiveEvent } | { gap: true } | { turn: TurnStatus };

/** Where a followed thread's turns stand on the node (`Live::status`): a loop
 * running, and turns accepted whose loop has not begun. */
export type TurnStatus = { running: boolean; queued: number };

/** A transcript entry, plus what only the client adds while a turn streams. */
export type Entry = TranscriptEntry | { kind: "note"; text: string } | { kind: "live-assistant"; text: string; /** In a room: who speaks (its `[Name · model]` note, design §2.27). */ speaker?: Speaker };
