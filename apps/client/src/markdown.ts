import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({ gfm: true, breaks: false });
// An image only by a URL the browser can load: a model's local path
// (`![](/tmp/x.jpg)`) would resolve against this node's origin, a 404 in
// the console; it shows as its path instead.
marked.use({
  renderer: {
    image({ href, text }) {
      if (/^(https?:|data:image\/)/i.test(href)) return false;
      const esc = (s: string) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
      return `<code title="${esc(text)}">${esc(href)}</code>`;
    },
  },
});
const cache = new Map<string, string>();

/** The most texts kept: more than a transcript's window or two, so a long
 * conversation does not parse every message again on each render. */
const CACHE_MAX = 2000;

/** Markdown to sanitized HTML (memoized, least recently used out: streaming
 * re-renders the same text often). */
export function render(text: string): string {
  let h = cache.get(text);
  if (h === undefined) {
    h = DOMPurify.sanitize(marked.parse(text, { async: false }) as string, { USE_PROFILES: { html: true } });
    if (cache.size >= CACHE_MAX) cache.delete(cache.keys().next().value!);
  } else cache.delete(text);
  cache.set(text, h);
  return h;
}
