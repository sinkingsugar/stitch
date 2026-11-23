// The sidebar's width: dragged by its right edge, kept in this browser.
import { signal } from "@preact/signals";

export const MIN_W = 260;
export const MAX_W = 640;
const KEY = "stitch.sidebar";

/** The default: wider on a wide window. */
export const defaultWidth = (win: number) => (win > 1400 ? 360 : 320);
export const clampWidth = (w: number) => Math.round(Math.min(MAX_W, Math.max(MIN_W, w)));

/** The stored width (a number in a string), else the default for the window. */
export function initialWidth(stored: string | null, win: number): number {
  const n = stored == null ? NaN : Number(stored);
  return Number.isFinite(n) ? clampWidth(n) : defaultWidth(win);
}

export const sideWidth = signal(initialWidth(localStorage.getItem(KEY), window.innerWidth));

export function setSideWidth(w: number, persist: boolean) {
  sideWidth.value = clampWidth(w);
  if (persist) localStorage.setItem(KEY, String(sideWidth.value));
}
export function resetSideWidth() {
  localStorage.removeItem(KEY);
  sideWidth.value = defaultWidth(window.innerWidth);
}

/** Where a card of `h` px goes vertically: level with its row, kept inside the window. */
export function cardTop(rowTop: number, h: number, vh: number, pad = 8): number {
  return Math.max(pad, Math.min(rowTop, vh - h - pad));
}

const COLLAPSE_KEY = "stitch.sidebar.collapsed";
/** The sidebar is folded away while the map is shown, so the map takes the whole width (kept in this browser). */
export const sideCollapsed = signal(localStorage.getItem(COLLAPSE_KEY) === "1");
export function toggleSide() {
  sideCollapsed.value = !sideCollapsed.value;
  localStorage.setItem(COLLAPSE_KEY, sideCollapsed.value ? "1" : "0");
}
