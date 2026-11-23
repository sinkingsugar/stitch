// The built-in dashboard on sample data, in a browser, with no node: what
// `live::fixture` serves the phone's screenshots, here for a desktop look
// (`bun tools/dashboard-preview.ts [port]`, after `just client`). The page is
// dist/dashboard/ with the apps bridge put first in its <head>, as the node
// does; live state is crates/stitch-node/src/apps/dashboard/fixture.json,
// its "@-N" N seconds ago and "@+N" N seconds on. `?native` acts as the
// phone's web view (Open links are `stitch://thread`).
const root = new URL("../../..", import.meta.url).pathname;
const dist = root + "apps/client/dist/dashboard/";
const apps = root + "crates/stitch-node/src/apps/";
const port = Number(process.argv[2] ?? 8932);

function fixture(): Record<string, unknown> {
  const text = require("fs").readFileSync(apps + "dashboard/fixture.json", "utf8") as string;
  const now = Date.now();
  return JSON.parse(text.replace(/"@([-+])(\d+)"/g, (_, s: string, n: string) => JSON.stringify(new Date(now + (s === "-" ? -1 : 1) * Number(n) * 1000).toISOString())));
}
const LIVE = { consent: "allowed", generation: 0, threads: 0, jobs: 0, attention: 0, schedules: 0 };

Bun.serve({
  port,
  async fetch(req) {
    const url = new URL(req.url);
    const p = url.pathname;
    const json = (v: unknown) => Response.json(v);
    if (p === "/" || p === "/index.html") {
      const start = { name: "Dashboard", version: "preview", pinned: true, state: {}, generation: 0, live: LIVE };
      const native = url.searchParams.has("native") ? "window.webkit={messageHandlers:{}};" : "";
      const page = (await Bun.file(dist + "index.html").text()).replace(
        "<head>",
        `<head><script>${native}window.__stitch=${JSON.stringify(start)};</script><script src="/_stitch/bridge.js"></script>`,
      );
      return new Response(page, { headers: { "Content-Type": "text/html; charset=utf-8" } });
    }
    if (p === "/app.js" || p === "/app.css") return new Response(Bun.file(dist + p.slice(1)));
    if (p === "/_stitch/bridge.js") return new Response(Bun.file(apps + "bridge.js"), { headers: { "Content-Type": "text/javascript" } });
    if (p === "/_stitch/events") return new Promise(() => {}); // the long poll: nothing moves
    if (p.startsWith("/_stitch/live/")) {
      const kind = p.slice("/_stitch/live/".length).replace("-", "_");
      if (req.method === "POST") return json({ ok: true });
      const d = fixture();
      return kind in d ? json(d[kind]) : new Response("not found", { status: 404 });
    }
    return new Response("not found", { status: 404 });
  },
});
console.log(`dashboard preview on http://localhost:${port}/`);
