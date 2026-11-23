// B0 spike: the cooperative loop over Chrome 144+'s consent attach (no
// extension). The watcher and a minimal overlay are injected into an
// isolated world and report through a CDP binding; the agent's input is
// marked with `timestamp: 1` (event.timeStamp 0 in the page). A simulated
// human (XTEST on the spike's Xvfb) types mid-batch: the agent pauses, its
// next step is refused (epoch moved), it resumes 4 s after the last input.
//   node consent-coop.mjs <user-data-dir>
// The Allow dialog must be clicked (human.py click 867 291 on the spike's
// display) while this waits.
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { spawn } from "node:child_process";

const DIR = path.dirname(new URL(import.meta.url).pathname);
const udd = process.argv[2];
const PORT = 8779;
const t0 = Date.now();
const ms = () => Date.now() - t0;
const log = (...a) => console.log(String(ms()).padStart(6), ...a);
const sleep = n => new Promise(r => setTimeout(r, n));
http.createServer((q, s) => { s.writeHead(200, { "content-type": "text/html" }); s.end(fs.readFileSync(path.join(DIR, "fixture.html"))); }).listen(PORT, "127.0.0.1");

const [port, p] = fs.readFileSync(`${udd}/DevToolsActivePort`, "utf8").trim().split("\n");
const ws = new WebSocket(`ws://127.0.0.1:${port}${p}`);
let next = 1;
const pending = new Map();
const listeners = [];
const call = (method, params = {}, sessionId) => new Promise((ok, err) => {
  const id = next++;
  pending.set(id, { ok, err });
  ws.send(JSON.stringify({ id, method, params, sessionId }));
});
ws.onmessage = e => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    const { ok, err } = pending.get(m.id);
    pending.delete(m.id);
    m.error ? err(new Error(JSON.stringify(m.error))) : ok(m.result);
  } else listeners.forEach(l => l(m));
};
await new Promise(r => (ws.onopen = r));
log("connected (after the Allow click)");

const WATCHER = `(() => {
  if (window.__stitch) return; window.__stitch = 1;
  for (const kind of ['pointerdown','keydown','wheel','paste'])
    addEventListener(kind, e => {
      if (!e.isTrusted || e.timeStamp === 0) return;
      const t = e.target; const d = t instanceof Element ? t.tagName.toLowerCase() + (t.id ? '#' + t.id : '') : '';
      stitchUserInput(JSON.stringify({ kind, target: d }));
    }, { capture: true, passive: true });
  const glow = () => { const h = document.createElement('stitch-overlay'); const r = h.attachShadow({ mode: 'closed' });
    r.innerHTML = '<div style="position:fixed;inset:0;pointer-events:none;z-index:2147483646;box-shadow:inset 0 0 0 3px rgba(150,90,255,.9)"></div>';
    document.documentElement.appendChild(h); };
  document.readyState === 'loading' ? addEventListener('DOMContentLoaded', glow) : glow();
})()`;

// A background tab of the agent's own: no tab group API here.
const { targetId } = await call("Target.createTarget", { url: "about:blank", background: true });
const { sessionId: s } = await call("Target.attachToTarget", { targetId, flatten: true });
const cdp = (m, prm) => call(m, prm, s);
await cdp("Page.enable");
await cdp("Runtime.enable");
await cdp("Runtime.addBinding", { name: "stitchUserInput", executionContextName: "stitch" });
await cdp("Page.addScriptToEvaluateOnNewDocument", { source: WATCHER, worldName: "stitch", runImmediately: true });
await cdp("Emulation.setFocusEmulationEnabled", { enabled: true });
let epoch = 0, lastUser = 0, paused = false;
const userEvents = [];
listeners.push(m => {
  if (m.sessionId === s && m.method === "Runtime.bindingCalled" && m.params.name === "stitchUserInput") {
    epoch++; paused = true; lastUser = Date.now();
    userEvents.push({ at: ms(), ...JSON.parse(m.params.payload) });
  }
});
await cdp("Page.navigate", { url: `http://127.0.0.1:${PORT}/` });
await sleep(1200);
await call("Target.activateTarget", { targetId }); // in front, so the simulated human can type into it
await sleep(500);
const ev = async e => (await cdp("Runtime.evaluate", { expression: e, returnByValue: true })).result.value;
const MARK = { timestamp: 1 };
async function focusField(id) {
  const r = await ev(`(() => { const r = document.getElementById('${id}').getBoundingClientRect(); return { x: r.left + 20, y: r.top + 8 }; })()`);
  for (const type of ["mousePressed", "mouseReleased"])
    await cdp("Input.dispatchMouseEvent", { type, x: r.x, y: r.y, button: "left", clickCount: 1, ...MARK });
  await ev(`document.getElementById('${id}').select()`);
}
setTimeout(async () => {
  const g = await ev(`(() => { const r = document.getElementById('notes').getBoundingClientRect();
    return { x: Math.round(window.screenX + r.left + 20), y: Math.round(window.screenY + (window.outerHeight - window.innerHeight) + r.top + 20) }; })()`);
  log("HUMAN clicks Notes and types");
  const env = { ...process.env, DISPLAY: ":99" };
  spawn("python3", [path.join(DIR, "human.py"), "click", g.x, g.y], { env })
    .on("exit", () => spawn("python3", [path.join(DIR, "human.py"), "type", "the user wrote this", "70"], { env }));
}, 1000);
const steps = [["name", "Ada Lovelace"], ["email", "ada@example.com"], ["notes", "Typed by the agent."]];
let seen = epoch, i = 0, refused = 0, pauseLatency = null;
const touched = new Set();
while (i < steps.length) {
  const [field, text] = steps[i];
  if (epoch !== seen) {
    refused++;
    log(`step ${i} (${field}) refused: epoch ${seen} -> ${epoch}; the user: ${JSON.stringify(userEvents.at(-1))}`);
    while (Date.now() - lastUser < 4000) await sleep(100);
    userEvents.forEach(e => { const m = e.target.match(/#(\w+)/); if (m) touched.add(m[1]); });
    seen = epoch; paused = false;
    log("resumed 4 s after the last user input; fresh read:", await ev("JSON.stringify(['name','email','notes'].map(i => document.getElementById(i).value))"));
    continue;
  }
  if (touched.has(field)) { log(`${field}: the user worked there; left as is`); i++; continue; }
  await focusField(field);
  let k = 0;
  for (; k < text.length && !paused; k++) {
    await cdp("Input.dispatchKeyEvent", { type: "keyDown", text: text[k], key: text[k], ...MARK });
    await cdp("Input.dispatchKeyEvent", { type: "keyUp", key: text[k], ...MARK });
    await sleep(60);
  }
  if (paused) { pauseLatency ??= ms() - userEvents[0].at; log(`${field}: stopped after ${k} chars`); continue; }
  i++;
}
log("final", await ev("JSON.stringify(['name','email','notes'].map(i => document.getElementById(i).value))"));
log(`user events ${userEvents.length} (expected 20: a click and 19 keys); refused ${refused}; pause noticed ≤ ${pauseLatency} ms after the first user event`);
await call("Target.closeTarget", { targetId });
process.exit(0);
