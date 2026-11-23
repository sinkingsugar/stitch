// Text measuring and wrapping for the map's canvas labels: the width per px of
// font size, cached; wrapping and ellipsis that never overflow.
export const FONT = '-apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, system-ui, sans-serif';

type Measure = (s: string) => number;
let measure: Measure = (s) => s.length * 0.56; // outside a browser (tests)
const wc = new Map<string, number>();

/** Use the browser's canvas for widths (called by the view). */
export function useCanvasMeasure() {
  const mc = document.createElement("canvas").getContext("2d");
  if (!mc) return;
  mc.font = "500 100px " + FONT;
  measure = (s) => mc.measureText(s).width / 100 * 1.03;
  wc.clear();
}
export function tw(s: string): number {
  let w = wc.get(s);
  if (w === undefined) {
    w = measure(s);
    if (wc.size > 20000) wc.clear();
    wc.set(s, w);
  }
  return w;
}
export function ellipsize(s: string, size: number, maxW: number): string {
  if (tw(s) * size <= maxW) return s;
  let lo = 0, hi = s.length;
  while (lo < hi) {
    const m = (lo + hi + 1) >> 1;
    if (tw(s.slice(0, m).trimEnd() + "…") * size <= maxW) lo = m; else hi = m - 1;
  }
  return lo ? s.slice(0, lo).trimEnd() + "…" : "";
}
/** Greedy wrap; `cut`: the text did not fit in `maxLines`. */
export function wrap(text: string, size: number, maxW: number, maxLines: number): { lines: string[]; cut: boolean } {
  const words = String(text || "").split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  let cur = "", i = 0;
  for (; i < words.length; i++) {
    const w = words[i]!, t = cur ? cur + " " + w : w;
    if (tw(t) * size <= maxW) { cur = t; continue; }
    if (cur) { lines.push(cur); cur = ""; if (lines.length >= maxLines) break; i--; continue; }
    lines.push(ellipsize(w, size, maxW));
    if (lines.length >= maxLines) { i++; break; }
  }
  let cut = i < words.length;
  if (!cut && cur) { if (lines.length < maxLines) lines.push(cur); else cut = true; }
  if (cut && lines.length) {
    const last = lines.length - 1;
    const rest = words.slice(i).join(" ");
    lines[last] = ellipsize(lines[last] + (rest ? " " + rest : ""), size, maxW);
    if (!lines[last]!.endsWith("…")) lines[last] = ellipsize(lines[last] + "…", size - 0.001, maxW);
  }
  return { lines, cut };
}
