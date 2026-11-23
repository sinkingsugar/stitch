// B0 spike: the node stub. Listens on the Unix socket the native host
// relays to, serves the fixture page, and runs one scenario:
//   node driver.mjs <scenario> [args]
// Scenarios: hello, bg, coop, infobar, alone, wait.
import net from "node:net";
import http from "node:http";
import fs from "node:fs";
import { execFileSync, spawn } from "node:child_process";
import path from "node:path";

const DIR = path.dirname(new URL(import.meta.url).pathname);
const SOCK = process.env.STITCH_B0_SOCK || "/tmp/stitch-b0/node.sock";
const OUT = "/tmp/stitch-b0/out";
fs.mkdirSync(OUT, { recursive: true });
const PORT = 8777;
// The agent marks its input: CDP `timestamp: 1` shows as event.timeStamp 0.
const MARK = process.env.MARK === "0" ? {} : { timestamp: 1 };
const t0 = Date.now();
const ms = () => Date.now() - t0;
const sleep = n => new Promise(r => setTimeout(r, n));
const log = (...a) => console.log(String(ms()).padStart(6), ...a);

// --- fixtures --------------------------------------------------------------
const server = http.createServer((req, res) => {
  const name = req.url.includes("user") ? "user" : "fixture";
  let html = fs.readFileSync(path.join(DIR, "fixture.html"), "utf8");
  if (name === "user") html = html.replace("B0 fixture form", "The user's own tab").replace("#2a7", "#c33");
  res.writeHead(200, { "content-type": "text/html" });
  res.end(html);
});
server.listen(PORT, "127.0.0.1");

// --- the link ---------------------------------------------------------------
const links = []; // one per connected profile
const waiters = [];
try { fs.unlinkSync(SOCK); } catch {}
const sock = net.createServer(c => {
  const link = { c, pending: new Map(), next: 1, events: [], hello: null, listeners: [] };
  let buf = "";
  c.on("data", d => {
    buf += d;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const msg = JSON.parse(buf.slice(0, i));
      buf = buf.slice(i + 1);
      if (msg.t === "res") {
        const p = link.pending.get(msg.id);
        link.pending.delete(msg.id);
        msg.ok ? p.ok(msg.result) : p.err(new Error(msg.error));
      } else if (msg.t === "hello") {
        link.hello = msg;
        links.push(link);
        log("hello", JSON.stringify({ profile: msg.profile, host_pid: msg.host_pid, sinceStart: msg.sinceStart }));
        waiters.splice(0).forEach(w => w());
      } else {
        link.events.push({ at: ms(), ...msg });
        link.listeners.forEach(l => l(msg));
      }
    }
  });
  c.on("close", () => log("link closed"));
});
sock.listen(SOCK);

function req(link, op, args = {}) {
  const id = link.next++;
  return new Promise((ok, err) => {
    link.pending.set(id, { ok, err });
    link.c.write(JSON.stringify({ t: "req", id, op, ...args }) + "\n");
  });
}
const cdp = (link, tab, method, params) => req(link, "cdp", { tab, method, params });

async function waitLinks(n, timeout = 60000) {
  const end = Date.now() + timeout;
  while (links.length < n) {
    if (Date.now() > end) throw new Error(`only ${links.length} of ${n} profiles connected`);
    await new Promise(r => { waiters.push(r); setTimeout(r, 500); });
  }
  return links;
}

// --- engine bits -------------------------------------------------------------
const INTERESTING = new Set(["button", "link", "textbox", "checkbox", "radio", "combobox", "searchbox",
  "menuitem", "tab", "heading", "slider", "switch", "option", "listbox"]);

async function snapshot(link, tab) {
  const t = Date.now();
  const { nodes } = await cdp(link, tab, "Accessibility.getFullAXTree", {});
  const refs = new Map();
  const lines = [];
  let n = 0;
  for (const node of nodes) {
    const role = node.role?.value;
    if (node.ignored || !INTERESTING.has(role) || node.backendDOMNodeId == null) continue;
    const ref = `ref_${++n}`;
    refs.set(ref, node.backendDOMNodeId);
    const name = (node.name?.value || "").trim();
    const value = node.value?.value;
    lines.push(`${role} "${name}"${value ? ` value="${value}"` : ""} [${ref}]`);
  }
  return { refs, text: lines.join("\n"), nodes: nodes.length, took: Date.now() - t };
}

function refOf(snap, name) {
  const m = snap.text.match(new RegExp(`"${name}"(?: value="[^"]*")? \\[(ref_\\d+)\\]`));
  return m && snap.refs.get(m[1]);
}

async function center(link, tab, backendNodeId) {
  await cdp(link, tab, "DOM.scrollIntoViewIfNeeded", { backendNodeId });
  const { quads } = await cdp(link, tab, "DOM.getContentQuads", { backendNodeId });
  const q = quads[0];
  return { x: (q[0] + q[2] + q[4] + q[6]) / 4, y: (q[1] + q[3] + q[5] + q[7]) / 4 };
}

async function click(link, tab, p) {
  await req(link, "overlay", { tab, x: p.x, y: p.y, label: "clicking" });
  for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) {
    await cdp(link, tab, "Input.dispatchMouseEvent", { type, x: p.x, y: p.y, button: "left", clickCount: 1, ...MARK });
  }
}

async function typeChar(link, tab, ch) {
  await cdp(link, tab, "Input.dispatchKeyEvent", { type: "keyDown", text: ch, key: ch, unmodifiedText: ch, ...MARK });
  await cdp(link, tab, "Input.dispatchKeyEvent", { type: "keyUp", key: ch, ...MARK });
}

async function screenshot(link, tab, file) {
  await req(link, "overlay", { tab, hide: true });
  const t = Date.now();
  const r = await Promise.race([
    cdp(link, tab, "Page.captureScreenshot", { format: "png" }),
    sleep(10000).then(() => null),
  ]);
  const took = Date.now() - t;
  await req(link, "overlay", { tab, hide: false });
  if (!r) return { took, timeout: true };
  fs.writeFileSync(file, Buffer.from(r.data, "base64"));
  return { took, bytes: r.data.length, file };
}

const evaluate = async (link, tab, expression) =>
  (await cdp(link, tab, "Runtime.evaluate", { expression, returnByValue: true })).result.value;

function rootShot(name) {
  const file = `${OUT}/${name}.png`;
  execFileSync("import", ["-display", ":99", "-window", "root", file]);
  return file;
}

// --- scenarios ----------------------------------------------------------------
const scenarios = {
  async hello() {
    const [l] = await waitLinks(1);
    log("tabs", JSON.stringify(await req(l, "tabs")));
  },

  // Does the MV3 service worker stay alive with the native port open and no
  // traffic? Requests a tab list every `step` seconds after long quiet gaps.
  async idle() {
    const [l] = await waitLinks(1);
    let closed = false;
    l.c.on("close", () => { closed = true; log("link closed: the service worker or host went away"); });
    for (const gap of [40, 120, 330]) {
      await sleep(gap * 1000);
      const t = Date.now();
      try { const tabs = await Promise.race([req(l, "tabs"), sleep(5000).then(() => null)]); log(`after ${gap} s quiet: ${tabs ? tabs.length + " tabs" : "NO ANSWER"} in ${Date.now() - t} ms, closed=${closed}`); }
      catch (e) { log(`after ${gap} s quiet: ${e.message}`); }
    }
  },

  async wait() {
    const n = Number(process.argv[3] || 1);
    await waitLinks(n);
    log("connected", links.map(l => l.hello.profile).join(", "));
  },

  // A background tab: snapshot and screenshots while the user's tab is in front.
  async bg() {
    const [l] = await waitLinks(1);
    const user = await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/user`, active: true });
    const agent = await req(l, "newTab", {
      url: `http://127.0.0.1:${PORT}/agent`, active: false,
      group: { title: "Stitch · B0", color: "purple" },
    });
    log("tabs", JSON.stringify({ user, agent }));
    await sleep(1500);
    let t = Date.now();
    await req(l, "attach", { tab: agent.tab });
    log("attach took", Date.now() - t);
    await sleep(500);
    rootShot("bg-attached");
    log("visibility before focus emulation", await evaluate(l, agent.tab, "document.visibilityState + ' hasFocus=' + document.hasFocus()"));
    const s1 = await screenshot(l, agent.tab, `${OUT}/bg-1-plain.png`);
    log("capture without focus emulation", JSON.stringify(s1));
    await cdp(l, agent.tab, "Emulation.setFocusEmulationEnabled", { enabled: true });
    log("visibility after focus emulation", await evaluate(l, agent.tab, "document.visibilityState + ' hasFocus=' + document.hasFocus()"));
    const s2 = await screenshot(l, agent.tab, `${OUT}/bg-2-focus.png`);
    log("capture with focus emulation", JSON.stringify(s2));
    // Keep it in the background a while, then change the page, then capture.
    await sleep(8000);
    await evaluate(l, agent.tab, "document.querySelector('h1').textContent = 'changed while hidden ' + Date.now(); 1");
    const s3 = await screenshot(l, agent.tab, `${OUT}/bg-3-after-8s.png`);
    log("capture after 8 s hidden + DOM change", JSON.stringify(s3), "clock now", await evaluate(l, agent.tab, "document.getElementById('clock').textContent"));
    const snap = await snapshot(l, agent.tab);
    log(`snapshot: ${snap.nodes} AX nodes, ${snap.refs.size} refs, ${snap.took} ms\n${snap.text}`);
    // Input in the background tab: click Name, type, read it back.
    const p = await center(l, agent.tab, refOf(snap, "Name"));
    await click(l, agent.tab, p);
    for (const ch of "hidden typing") await typeChar(l, agent.tab, ch);
    log("value typed in background tab:", await evaluate(l, agent.tab, "document.getElementById('name').value"),
      "| user tab still active:", JSON.stringify((await req(l, "tabs")).filter(x => x.active)));
    const s4 = await screenshot(l, agent.tab, `${OUT}/bg-4-typed.png`);
    log("capture after typing", JSON.stringify(s4));
    const u = links[0].events.filter(e => e.t === "user_input");
    log("user_input events during agent-only work (should be 0):", u.length, JSON.stringify(u));
    // Screencast frames as the alternative.
    let frames = 0;
    l.listeners.push(m => { if (m.method === "Page.screencastFrame") { frames++; cdp(l, agent.tab, "Page.screencastFrameAck", { sessionId: m.params.sessionId }); if (frames === 1) fs.writeFileSync(`${OUT}/bg-5-screencast.jpg`, Buffer.from(m.params.data, "base64")); } });
    await cdp(l, agent.tab, "Page.startScreencast", { format: "jpeg", quality: 70, everyNthFrame: 1 });
    await sleep(3000);
    await cdp(l, agent.tab, "Page.stopScreencast", {});
    log("screencast frames in 3 s from a background tab:", frames);
    t = Date.now();
    await req(l, "detach", { tab: agent.tab });
    log("detach took", Date.now() - t);
    await sleep(300);
    rootShot("bg-detached");
  },

  // The infobar: when it shows and goes away.
  async infobar() {
    const [l] = await waitLinks(1);
    const a = await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/agent`, active: true, group: { title: "Stitch · B0" } });
    await sleep(1000);
    rootShot("infobar-0-before");
    await req(l, "attach", { tab: a.tab });
    await sleep(300); rootShot("infobar-1-attached-300ms");
    await req(l, "detach", { tab: a.tab });
    await sleep(300); rootShot("infobar-2-detached-300ms");
    await sleep(3000); rootShot("infobar-3-detached-3s");
    await req(l, "attach", { tab: a.tab });
    await sleep(300);
    const b = await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/user`, active: true });
    await sleep(800); rootShot("infobar-4-other-tab-in-front");
    await req(l, "detach", { tab: a.tab });
  },

  // How long the infobar stays after a detach (the pixel row where it shows).
  async infobar2() {
    const [l] = await waitLinks(1);
    const tabs = await req(l, "tabs");
    const a = tabs.find(t => t.url.includes("/agent"))?.id ?? (await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/agent`, active: true })).tab;
    await req(l, "activate", { tab: a });
    await sleep(500);
    const bar = () => { const f = rootShot("ib"); const px = execFileSync("magick", [f, "-format", "%[pixel:p{860,115}]", "info:"]).toString(); return px; };
    log("before attach, pixel at the bar's button:", bar());
    await req(l, "attach", { tab: a });
    await sleep(500);
    log("attached:", bar());
    await req(l, "detach", { tab: a });
    const t = Date.now();
    for (let i = 0; i < 30; i++) { log(`+${Date.now() - t} ms after detach:`, bar()); await sleep(500); }
  },

  async cancelbar() {
    const [l] = await waitLinks(1);
    const tabs = await req(l, "tabs");
    const a = tabs.find(t => t.url.includes("/agent")).id;
    await req(l, "attach", { tab: a });
    await sleep(800);
    execFileSync("python3", [path.join(DIR, "human.py"), "click", "860", "115"], { env: { ...process.env, DISPLAY: ":99" } });
    await sleep(800);
    log("events", JSON.stringify(l.events.filter(e => e.t === "detached")));
    try { await cdp(l, a, "Runtime.evaluate", { expression: "1" }); log("still attached?!"); } catch (e) { log("after Cancel:", e.message); }
  },

  // The infobar's ×: does dismissing it keep the debugger, and does a later
  // attach show it again?
  async closebar() {
    const [l] = await waitLinks(1);
    const a = (await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/agent`, active: true })).tab;
    await sleep(1200);
    await req(l, "attach", { tab: a });
    await sleep(800);
    const env = { ...process.env, DISPLAY: ":99" };
    execFileSync("python3", [path.join(DIR, "human.py"), "click", "1370", "115"], { env });
    await sleep(800);
    const px = () => execFileSync("magick", [rootShot("cb"), "-format", "%[pixel:p{1370,115}]", "info:"]).toString();
    log("after ×: bar pixel", px(), "events", JSON.stringify(l.events.filter(e => e.t === "detached")));
    try { await cdp(l, a, "Runtime.evaluate", { expression: "1" }); log("still attached after ×"); } catch (e) { log("after ×:", e.message); }
    const b = (await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/agent?b`, active: true })).tab;
    await sleep(1200);
    await req(l, "attach", { tab: b });
    await sleep(800);
    log("second tab attached while the first still is: bar pixel", px());
    await req(l, "detach", { tab: b });
    await req(l, "detach", { tab: a });
    await sleep(6000);
    await req(l, "attach", { tab: a });
    await sleep(800);
    log("re-attach after all detached + 6 s: bar pixel", px());
    await req(l, "detach", { tab: a });
  },

  // Attribution under overlap: the agent types without pausing while a
  // human types elsewhere; how many human keys does the watcher see?
  async race() {
    const [l] = await waitLinks(1);
    const a = await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/agent`, active: true, group: { title: "Stitch · race" } });
    await sleep(1500);
    await req(l, "attach", { tab: a.tab });
    await sleep(800);
    const seen = [];
    l.listeners.push(m => { if (m.t === "user_input" && m.tab === a.tab) seen.push(m); });
    const snap = await snapshot(l, a.tab);
    await click(l, a.tab, await center(l, a.tab, refOf(snap, "Email")));
    const geo = await evaluate(l, a.tab, `(() => { const r = document.getElementById('notes').getBoundingClientRect();
      return { x: Math.round(window.screenX + r.left + 20), y: Math.round(window.screenY + (window.outerHeight - window.innerHeight) + r.top + 20) }; })()`);
    const env = { ...process.env, DISPLAY: ":99" };
    const text = "abcdefghijklmnopqrstuvwxyzabcdefghijklmn"; // 40 keys, 25 ms apart (a fast typist)
    const agentText = "x".repeat(150);
    const human = new Promise(r => setTimeout(() => spawn("python3", [path.join(DIR, "human.py"), "type", text, "25"], { env }).on("exit", r), 300));
    for (const ch of agentText) { await typeChar(l, a.tab, ch); await sleep(10); }
    await human;
    await sleep(500);
    const keys = seen.filter(m => m.kind === "keydown").length;
    const vals = JSON.parse(await evaluate(l, a.tab, "JSON.stringify([document.getElementById('email').value.length, document.getElementById('notes').value])"));
    log(`human keys reported as the user's: ${keys} of ${text.length}; agent chars landed ${vals[0]}; text the human produced: ${JSON.stringify(vals[1])}`);
    await req(l, "detach", { tab: a.tab });
  },

  // Snapshot sources on real sites: CDP's full AX tree (filtered to
  // interesting roles) versus a DOM walk of the kind a content script does
  // (run here through Runtime.evaluate, as the extension's script would).
  async snap() {
    const [l] = await waitLinks(1);
    const sites = ["https://en.wikipedia.org/wiki/Ada_Lovelace", "https://github.com/microsoft/playwright",
      "https://news.ycombinator.com/", "https://www.amazon.com/", "https://developer.mozilla.org/en-US/docs/Web/HTML",
      "https://www.bbc.com/news"];
    const WALK = `(() => {
      const t = performance.now(); const out = []; let n = 0;
      const role = e => e.getAttribute('role') || ({A: e.hasAttribute('href') ? 'link' : '', BUTTON: 'button', INPUT: ({checkbox:'checkbox', radio:'radio', submit:'button', button:'button'})[e.type] || 'textbox', TEXTAREA: 'textbox', SELECT: 'combobox', H1:'heading', H2:'heading', H3:'heading'})[e.tagName] || '';
      const vis = e => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(e).visibility !== 'hidden'; };
      const w = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
      for (let e = w.currentNode; e; e = w.nextNode()) { n++; const r = role(e); if (!r || !vis(e)) continue;
        const name = (e.getAttribute('aria-label') || e.innerText || e.value || e.placeholder || e.title || '').trim().replace(/\\s+/g, ' ').slice(0, 80);
        out.push(r + ' "' + name + '"'); }
      return JSON.stringify({ took: Math.round(performance.now() - t), elements: n, lines: out.length, chars: out.join('\\n').length });
    })()`;
    const tab = (await req(l, "newTab", { url: "about:blank", active: false, group: { title: "Stitch · snap" } })).tab;
    await req(l, "attach", { tab });
    for (const url of sites) {
      await cdp(l, tab, "Page.navigate", { url });
      await sleep(6000);
      const ax = await snapshot(l, tab);
      const walk = JSON.parse(await evaluate(l, tab, WALK));
      log(`${url}\n   CDP AX: ${ax.nodes} nodes -> ${ax.refs.size} refs, ${ax.text.length} chars, ${ax.took} ms | DOM walk: ${walk.elements} elements -> ${walk.lines} lines, ${walk.chars} chars, ${walk.took} ms`);
    }
    await req(l, "detach", { tab });
  },

  // The overlay and the infobar, on a tab in front, attached.
  async look() {
    const [l] = await waitLinks(1);
    const a = await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/agent`, active: true, group: { title: "Stitch · look", color: "purple" } });
    await sleep(1500);
    await req(l, "attach", { tab: a.tab });
    log("overlay reply", JSON.stringify(await req(l, "overlay", { tab: a.tab, x: 300, y: 250, label: "clicking Name", state: "Stitch is filling the form" })));
    log("in DOM:", await evaluate(l, a.tab, "!!document.querySelector('stitch-overlay')"));
    await sleep(1000);
    rootShot("look-attached");
    const s = await screenshot(l, a.tab, `${OUT}/look-capture.png`);
    log("capture", JSON.stringify(s));
    await req(l, "detach", { tab: a.tab });
    await sleep(1000);
    rootShot("look-detached");
  },

  // Can the page tell CDP input from the user's by a marker the agent sets?
  // Records event.timeStamp, code, sourceCapabilities for agent keys/clicks
  // (with and without a CDP `timestamp`) and for real X input.
  async mark() {
    const [l] = await waitLinks(1);
    const a = await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/agent`, active: true, group: { title: "Stitch · mark" } });
    await sleep(1500);
    await req(l, "attach", { tab: a.tab });
    await evaluate(l, a.tab, `window.__ev = []; for (const k of ['keydown','pointerdown']) addEventListener(k, e => __ev.push({k, key: e.key, code: e.code, ts: e.timeStamp, now: performance.now(), sc: e.sourceCapabilities ? 'yes' : 'null', pid: e.pointerId, pt: e.pointerType, x: e.screenX, y: e.screenY}), true); 1`);
    const snap = await snapshot(l, a.tab);
    const p = await center(l, a.tab, refOf(snap, "Email"));
    await click(l, a.tab, p);
    await cdp(l, a.tab, "Input.dispatchKeyEvent", { type: "keyDown", text: "a", key: "a", code: "KeyA" });
    await cdp(l, a.tab, "Input.dispatchKeyEvent", { type: "keyUp", key: "a", code: "KeyA" });
    await cdp(l, a.tab, "Input.dispatchKeyEvent", { type: "keyDown", text: "b", key: "b", code: "KeyB", timestamp: 1 });
    await cdp(l, a.tab, "Input.dispatchKeyEvent", { type: "keyUp", key: "b", code: "KeyB", timestamp: 1 });
    for (const type of ["mousePressed", "mouseReleased"])
      await cdp(l, a.tab, "Input.dispatchMouseEvent", { type, x: p.x, y: p.y, button: "left", clickCount: 1, timestamp: 1 });
    const geo = await evaluate(l, a.tab, `(() => { const r = document.getElementById('email').getBoundingClientRect();
      return { x: Math.round(window.screenX + r.left + 40), y: Math.round(window.screenY + (window.outerHeight - window.innerHeight) + r.top + 5) }; })()`);
    const env = { ...process.env, DISPLAY: ":99" };
    execFileSync("python3", [path.join(DIR, "human.py"), "click", String(geo.x), String(geo.y)], { env });
    execFileSync("python3", [path.join(DIR, "human.py"), "type", "c", "30"], { env });
    await sleep(300);
    log("events\n" + (await evaluate(l, a.tab, "__ev.map(e => JSON.stringify(e)).join('\\n')")));
    await req(l, "detach", { tab: a.tab });
  },

  // The cooperative loop: the agent fills the form in its tab, in front; a
  // simulated human types mid-batch; the agent pauses, its next action is
  // refused (epoch moved), it waits for 4 s of quiet, re-reads, resumes.
  async coop() {
    const [l] = await waitLinks(1);
    const a = await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/agent`, active: true, group: { title: "Stitch · B0 coop", color: "purple" } });
    await sleep(1500);
    await req(l, "attach", { tab: a.tab });
    await cdp(l, a.tab, "Emulation.setFocusEmulationEnabled", { enabled: true });
    await req(l, "overlay", { tab: a.tab, state: "Stitch is filling the form" });
    let epoch = 0, seen = 0, paused = false, lastUser = 0;
    const userEvents = [];
    l.listeners.push(m => {
      if (m.t === "user_input" && m.tab === a.tab) {
        epoch++; paused = true; lastUser = Date.now();
        userEvents.push({ at: ms(), kind: m.kind, target: m.target });
      }
      if (m.t === "stop") { log("STOP pressed"); process.exit(0); }
    });
    let snap = await snapshot(l, a.tab); seen = epoch;
    const ref = name => refOf(snap, name);
    // Each step types into a field by ref (focusing it first), so a resumed
    // step never types wherever the user left the focus.
    const steps = [
      { field: "Name", text: "Ada Lovelace" },
      { field: "Email", text: "ada@example.com" },
      { field: "Notes", text: "Typed by the agent." },
    ];
    // The human: at 1.0 s, clicks the Notes field (measured now: the infobar
    // moves the page) and types.
    setTimeout(async () => {
      const geo = await evaluate(l, a.tab, `(() => { const r = document.getElementById('notes').getBoundingClientRect();
        return { x: Math.round(window.screenX + r.left + 20), y: Math.round(window.screenY + (window.outerHeight - window.innerHeight) + r.top + 20) }; })()`);
      log("HUMAN clicks Notes at screen", JSON.stringify(geo), "and types");
      const env = { ...process.env, DISPLAY: ":99" };
      spawn("python3", [path.join(DIR, "human.py"), "click", geo.x, geo.y], { env })
        .on("exit", () => spawn("python3", [path.join(DIR, "human.py"), "type", "the user wrote this", "70"], { env }));
    }, 1000);
    const touched = new Set();
    let i = 0, refused = 0, firstUserAt = null, pausedAt = null, agentKeysAfterPause = 0;
    while (i < steps.length) {
      const step = steps[i];
      if (epoch !== seen) {
        refused++;
        log(`step ${i} (${step.field}) REFUSED: epoch ${seen} -> ${epoch}; the user: ${JSON.stringify(userEvents.at(-1))}`);
        await req(l, "overlay", { tab: a.tab, state: "Paused: you are using this tab" });
        while (Date.now() - lastUser < 4000) await sleep(100);
        snap = await snapshot(l, a.tab); seen = epoch; paused = false;
        userEvents.forEach(e => { const m = e.target.match(/#(\w+)/); if (m) touched.add(m[1]); });
        log("resumed after 4 s idle; fresh snapshot:\n" + snap.text);
        await req(l, "overlay", { tab: a.tab, state: "Stitch is filling the form" });
        continue;
      }
      if (touched.has(step.field.toLowerCase())) { log(`${step.field}: the user worked there; left as they made it`); i++; continue; }
      await click(l, a.tab, await center(l, a.tab, ref(step.field)));
      // Select what the agent typed before (a resumed step starts over).
      await cdp(l, a.tab, "Runtime.evaluate", { expression: `document.getElementById('${step.field.toLowerCase()}').select()` });
      let k = 0;
      for (; k < step.text.length; k++) {
        if (paused) break;
        await typeChar(l, a.tab, step.text[k]);
        await sleep(60);
      }
      if (paused) {
        if (!firstUserAt) { firstUserAt = userEvents[0].at; pausedAt = ms(); }
        log(`step stopped in "${step.text}" after ${k} chars`);
        continue;
      }
      i++;
    }
    const values = await evaluate(l, a.tab, "JSON.stringify(['name','email','notes'].map(id => document.getElementById(id).value))");
    log("final values", values);
    log("user events", JSON.stringify(userEvents.slice(0, 3)), "total", userEvents.length);
    log(`refused steps ${refused}; pause latency ≈ ${pausedAt - firstUserAt} ms after the first user event`);
    rootShot("coop-final");
    await req(l, "detach", { tab: a.tab });
  },

  // Agent-alone: a separate unfocused window; the group moves between windows.
  async alone() {
    const [l] = await waitLinks(1);
    const user = await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/user`, active: true });
    const w = await req(l, "newWindow", { url: `http://127.0.0.1:${PORT}/agent`, focused: false });
    log("agent window", JSON.stringify(w));
    await sleep(1500);
    const tabs = await req(l, "tabs");
    log("tabs", JSON.stringify(tabs));
    await req(l, "attach", { tab: w.tab });
    await cdp(l, w.tab, "Emulation.setFocusEmulationEnabled", { enabled: true });
    const s = await screenshot(l, w.tab, `${OUT}/alone-1.png`);
    log("capture in agent-alone window", JSON.stringify(s));
    rootShot("alone-root");
    // Move into the user's window as a group, and back.
    const g = await req(l, "cdp", { tab: w.tab, method: "Runtime.evaluate", params: { expression: "1" } });
    const groupTab = await req(l, "newTab", { url: `http://127.0.0.1:${PORT}/agent?2`, window: w.window, group: { title: "Stitch · alone" } });
    await req(l, "moveGroup", { group: groupTab.group, window: user.window });
    log("group moved into the user's window; tabs", JSON.stringify((await req(l, "tabs")).map(t => [t.id, t.window, t.group])));
    await req(l, "detach", { tab: w.tab });
  },
};

const name = process.argv[2] || "hello";
scenarios[name]()
  .then(() => { log("done"); process.exit(0); })
  .catch(e => { log("FAILED", e.stack || e); process.exit(1); });
