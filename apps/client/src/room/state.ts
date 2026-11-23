// Rooms' actions in the web client (design §2.27): creating one (`+ Room`,
// "Pull in model…", the map's "Open room"), changing its roster and policy
// (`RoomUpdate`), and what the New Room dialog is opened with.
import { signal } from "@preact/signals";
import * as S from "../store";
import * as A from "../actions/state";
import type { RoomInfo } from "../payload";
import type { StagedScope } from "../scopes";
import { applyChange, infoOf, roomCreate, roomUpdate, type NewRoom, type RoomChange } from "./logic";

/** What the New Room dialog opens with. */
export interface RoomSeed {
  /** "Pull in model…": the 1:1 thread the room continues. */
  from?: { id: string; title: string } | null;
  /** Threads and efforts the room is opened on. */
  scopes?: StagedScope[];
  /** A 1:1 thread the dialog may pull in instead (the map's card, the menu). */
  pullable?: { id: string; title: string } | null;
  /** Threads offered as participants (the map's picks), unchecked. */
  offer?: { id: string; title: string }[];
  /** Checked from the start. */
  invite?: string[];
  /** A place's label or key. */
  place?: string | null;
}

/** The New Room dialog, open with its seed. */
export const newRoom = signal<RoomSeed | null>(null);
/** A `@slug` for the open room's composer to insert at its caret (a pill's `@`). */
export const mention = signal<string | null>(null);

/** Opens the New Room dialog (the places read for its picker). */
export function openNewRoom(seed: RoomSeed = {}) {
  A.menuOpen.value = false;
  newRoom.value = seed;
  void S.loadPlaces();
}

/** "Pull in model…" on a 1:1 conversation. */
export const pullIn = (t: { id: string; title: string; place?: string }) => openNewRoom({ from: { id: t.id, title: t.title }, place: t.place ?? null });

/** The place key a seed's place names (a label or a key; none: global). */
export function placeKey(place: string | null | undefined): string | null {
  if (!place || place === "global") return null;
  const p = S.places.value.find((x) => x.label === place || x.key === place);
  return p?.key ?? null;
}

/** Creates the room and opens it. */
export const create = (r: NewRoom) =>
  A.run(r.from ? "pulling in (a room of threads)" : "making the room", async () => {
    const info = infoOf(await S.rpc(roomCreate(r)));
    newRoom.value = null;
    await S.refresh();
    await S.select(info.room);
    S.room.value = info;
    return info;
  });

/** Changes the open room's roster or policy: shown at once, then as the node says. */
export async function update(room: string, c: RoomChange): Promise<RoomInfo | undefined> {
  const was = S.room.value;
  if (was && was.room === room) S.room.value = applyChange(was, c);
  const info = await A.run("changing the room", async () => infoOf(await S.rpc(roomUpdate(room, c))));
  if (S.selected.value !== room) return info;
  if (info) S.room.value = info;
  else if (was) S.room.value = was;
  if (info && (c.add?.length || c.remove?.length)) void S.refresh();
  return info;
}

/** Stops one participant's turn (its own thread's Cancel), the round goes on. */
export const stopOne = (thread: string) => A.run("stopping", () => S.rpc({ Cancel: { thread } }));
