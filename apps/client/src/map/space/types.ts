// What Space (the map in 3D, design §2.25) and the rest of the client share: types only, so the main bundle and
// the lazily loaded chunk (`space.js`) agree on them without sharing code.
import type { MFilter, Model, Sel, Stats } from "../model";

export interface Circle { x: number; y: number; r: number }
/** Islands' circle packing, in its world units (0…size, y down): where every place and effort is. Space stands on it. */
export interface PackLayout { size: number; places: Map<string, Circle>; efforts: Map<string, Circle> }
/** What the 2D view is looking at, in pack units: the centre and the width shown. */
export interface View2D { x: number; y: number; w: number }

export interface SpaceIO {
  model(): Model;
  stats(): Stats;
  filter(): MFilter;
  sel(): Sel | null;
  hovered(): Sel | null;
  /** The thread open in the client: marked. */
  opened(): string | null;
  /** Islands' packing (null: nothing visible). */
  layout(): PackLayout | null;
  select(s: Sel | null): void;
  hover(s: Sel | null): void;
  open(s: Sel): void;
  /** The IDs of the efforts and threads in the map's scope tray (design §2.27): ringed. A plain set, handed over
   * (never the signal: the chunk imports no signals). */
  picked?(): ReadonlySet<string>;
  /** Shift+click (a tap, not a shift-drag pan) on an effort or a thread: in or out of the tray; the camera stays. */
  togglePick?(s: Sel): void;
  /** No animations (screenshots). */
  instant: boolean;
  /** Width the detail card takes on the right when something is selected (0: none, a narrow screen). */
  cardWidth(): number;
}

export interface SpaceStats { drawCalls: number; triangles: number; points: number; nodes: number; edges: number; edgeInstances: number; labels: number; frameMs: number; frames: number }

export interface Space {
  /** New data or a re-packing (`data`: re-layout, gliding), or a restyle (`filter`). */
  update(kind: "data" | "filter"): void;
  /** The selection changed; `from` is where (a selection from elsewhere flies there). */
  selected(from: string): void;
  /** Something only drawn changed (hover, the open conversation). */
  redraw(): void;
  /** Shown. `from2d`: coming from 2D, what Islands was looking at (null: everything), to start above it;
   * undefined: as it was (back from a conversation). */
  show(from2d?: View2D | null): void;
  hide(): void;
  destroy(): void;
  topDown(): void;
  /** The tilted overview of everything. */
  overview(): void;
  /** What the camera looks at, in pack units (for going back to 2D). */
  target(): { x: number; y: number };
  debug: { stats(): SpaceStats; camera(): { target: [number, number, number]; dist: number; theta: number; phi: number }; pick(x: number, y: number): Sel | null; labels(): string[]; probe?(): unknown };
}

/** The chunk's module (`space.js`). */
export interface SpaceModule { createSpace(host: HTMLElement, io: SpaceIO): Space }
