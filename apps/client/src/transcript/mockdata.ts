// `?mock`: one of each rich entry (images, an app, diffs).
import type { Entry } from "../model";
import { mockBlobs } from "./blob";

const svg = (bg: string, fg: string, label: string) =>
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="400"><rect width="640" height="400" fill="${bg}"/>` +
      `<circle cx="470" cy="150" r="90" fill="${fg}" opacity=".35"/><rect x="60" y="230" width="360" height="40" rx="8" fill="${fg}" opacity=".6"/>` +
      `<text x="60" y="120" font-family="sans-serif" font-size="44" fill="#fff">${label}</text></svg>`,
  );
const img = (id: string, bg: string, fg: string, label: string, name: string) => {
  const hash = "blake3:mock-" + id;
  mockBlobs.set(hash, svg(bg, fg, label));
  return { hash, type: "image/svg+xml", name };
};

export const MOCK_APP_URL =
  "data:text/html;charset=utf-8," +
  encodeURIComponent(
    `<body style="margin:0;font:15px system-ui;background:#10243a;color:#e6f2ff;display:grid;place-items:center;height:100vh">` +
      `<div style="text-align:center"><h2>Hello from an app</h2><p>on its own origin, sandboxed.</p>` +
      `<button onclick="this.textContent='clicked '+(++window.n||(window.n=1))">click me</button></div></body>`,
  );

const diffText = `@@ -12,7 +12,9 @@ fn connect() {
   let ws = WebSocket::new(endpoint());
-  ws.onclose = |_| retry(250);
+  ws.onclose = |_| {
+    backoff = (backoff * 2).min(5000);
+    retry(backoff);
+  };
   ws.onopen = hello;
 }`;

export function richEntries(start: number): Entry[] {
  let n = start;
  const at = new Date(Date.now() - 300_000).toISOString();
  const tool = (name: string, call: string, output: string, extra: object = {}): Entry =>
    ({ n: n++, kind: "tool", at, name, call, result: { ok: true, output }, images: [], app: null, diff: null, ...extra }) as Entry;
  const shot = img("shot", "#1e3a5f", "#61c5c5", "Dashboard", "screenshot.png");
  return [
    { n: n++, kind: "user", from: null, at, text: "Here is the mock-up I mean:", images: [img("mock", "#3b2a4a", "#f5a05a", "Mock-up", "mockup.png"), img("mock2", "#2a3b4a", "#ef706d", "Variant B", "variant-b.png")] },
    tool("view_file", "view_file /tmp/shot.png", "(image)", { images: [shot] }),
    tool("edit_file", "edit_file crates/stitch-node/src/conn.rs", "edited conn.rs (1 replacement)", { diff: { path: "crates/stitch-node/src/conn.rs", text: diffText } }),
    tool("write_file", "write_file notes/plan.md", "created notes/plan.md (3 lines)", { diff: { path: "notes/plan.md", text: "@@ -0,0 +1,3 @@\n+# Plan\n+- ship it\n+- then rest" } }),
    tool("show_app", "show_app {\"path\":\"~/demo\",\"name\":\"Demo\"}", "Shown: app \"Demo\"", { app: { manifest: "blake3:mock-app", name: "Demo" } }),
    { n: n++, kind: "assistant", from: null, at, text: "Generated this for you:", images: [img("gen", "#183a2e", "#61c5c5", "Generated", "image-m3-1.png")] },
  ];
}
