// Wording helpers ported from the dashboard (apps/dashboard/app.js).
import { signal } from "@preact/signals";
import type { ThreadItem } from "../model";
import { blocked } from "./state";

/** Ticks every second while something that shows elapsed time is mounted. */
export const now = signal(Date.now());
let ticking = 0;
let timer: ReturnType<typeof setInterval> | undefined;
export function useTick(): () => void {
  if (ticking++ === 0) timer = setInterval(() => (now.value = Date.now()), 1000);
  return () => { if (--ticking === 0) clearInterval(timer); };
}

export function ago(iso: string | null | undefined): string {
  if (!iso) return "";
  const s = Math.max(0, (now.value - Date.parse(iso)) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return Math.round(s / 60) + " min ago";
  if (s < 86400) return Math.round(s / 3600) + " h ago";
  return Math.round(s / 86400) + " d ago";
}
export function short(iso: string | null | undefined): string {
  const t = ago(iso);
  return t === "just now" ? "now" : t.replace(" min ago", "m").replace(" h ago", "h").replace(" d ago", "d");
}
export function until(iso: string): string {
  const s = (Date.parse(iso) - now.value) / 1000;
  if (s < -60) return "due";
  if (s < 60) return "now";
  if (s < 3600) return "in " + Math.round(s / 60) + " min";
  if (s < 86400) return "in " + Math.round(s / 3600) + " h";
  return "in " + Math.round(s / 86400) + " d";
}
export function hhmm(iso: string): string {
  return new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}
export function when(iso: string): string {
  const d = new Date(iso);
  return new Date().toDateString() === d.toDateString()
    ? hhmm(iso)
    : d.toLocaleString([], { weekday: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}
export function dayOf(iso: string): string {
  const d = new Date(iso);
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((start(new Date()) - start(d)) / 86400e3);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  return d.toLocaleDateString([], { weekday: "long", day: "numeric", month: "short" });
}
/** "3m12s": a call's elapsed time. */
export function spoken(iso: string): string {
  const s = Math.max(0, Math.floor((now.value - Date.parse(iso)) / 1000));
  if (s < 60) return s + "s";
  const two = (n: number) => String(n).padStart(2, "0");
  if (s < 3600) return Math.floor(s / 60) + "m" + two(s % 60) + "s";
  return Math.floor(s / 3600) + "h" + two(Math.floor((s % 3600) / 60)) + "m";
}
export const money = (c: number | null | undefined) => (c ? `$${c.toFixed(2)}` : "");

export type Dot = "working" | "waiting" | "failed" | "done";
export function dotOf(t: ThreadItem): Dot {
  if (t.question) return "waiting";
  // Blocked counts while it is on the needs list: a dismissed one is not the user's any more.
  if ((t.state === "blocked" || t.outcome === "blocked") && blocked.value.some((x) => x.id === t.id)) return "waiting";
  if (t.state === "blocked" || t.outcome === "blocked") return t.state === "working" ? "working" : "done";
  if (t.state === "working") return "working";
  if (["failed", "not-done", "stopped", "over-budget", "unconfirmed"].includes(t.outcome ?? "")) return "failed";
  return "done";
}
export const lastOf = (t: ThreadItem) => t.last_step || t.started || "";

export function sentence(text: string | null | undefined, max = 160): string {
  if (!text) return "";
  const one = text.replace(/\s+/g, " ").replace(/[*`#]/g, "").trim();
  const m = one.match(/^(.{20,}?[.!?])(\s|$)/);
  const s = m ? m[1]! : one;
  return s.length > max ? s.slice(0, max - 1) + "…" : s;
}

// A spawned thread's title is often its goal cut at 80 characters: make a short
// title of the goal's first sentence that says what to do.
const BOILER = /^(you are|you're|you work|do this|do the work|do exactly|do not|don't|never|only change|change nothing|work in|work directly|follow|read |start with|every earlier|ignore|nobody|the earlier|keep it|this is a|be direct)/i;
function cleanSentence(s: string): string {
  let t = s;
  for (let i = 0; i < 3; i++) t = t.replace(/\s*\([^()]*\)/g, "");
  t = t.replace(/^(read-only|test only|keep it tiny)[^a-z0-9]*\s*/i, "");
  t = t.replace(/^(using the web|in [^,;:]{1,70}?|on [^,;:]{1,40}?)[,:;]\s+/i, "");
  const cut = t.search(/[:;]\s/);
  if (cut > 24) t = t.slice(0, cut);
  t = t.replace(/\*\*|`/g, "").replace(/[.!\s]+$/, "").trim();
  if (t.length > 110) t = t.slice(0, 110).replace(/\s+\S*$/, "") + "…";
  return t ? t[0]!.toUpperCase() + t.slice(1) : "";
}
export function titleOf(t: ThreadItem): string {
  const title = (t.title || "").trim();
  const stem = title.replace(/…$/, "");
  const fromGoal = !title || (t.goal || "").replace(/\s+/g, " ").startsWith(stem.replace(/\s+/g, " "));
  if (!fromGoal) return title;
  const goal = (t.goal || title).replace(/\s+/g, " ").replace(/\s*…$/, "").trim();
  const sentences = goal.split(/(?<=[.!?])\s+(?=[A-Z`"*~(])/);
  for (const s of sentences) {
    const stripped = s.replace(/^(read-only|test only)[^a-z0-9]*\s*/i, "");
    if (BOILER.test(stripped)) continue;
    const c = cleanSentence(s);
    if (c.split(/\s+/).length >= 3) return c;
  }
  return cleanSentence(sentences[0] || goal) || title;
}

/** What came of a thread, short. */
export function outcomeOf(t: ThreadItem): string {
  const words = sentence(t.last_words);
  if (t.state === "working") return words || "working…";
  if (t.question) return `waits for you to run ${t.question.command}`;
  const how = t.state === "cancelled" ? "cancelled" : t.outcome || t.said || "";
  if (how === "done") return words || "done";
  return how.replace(/-/g, " ") + (words ? ": " + words : "");
}
