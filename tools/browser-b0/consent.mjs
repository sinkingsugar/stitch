// B0 spike: Chrome 144+'s consent attach. Reads DevToolsActivePort from the
// user-data-dir, connects to the browser WebSocket, and times how long the
// first command waits (the Allow dialog), then lists what it can reach.
//   node consent.mjs <user-data-dir> [hold seconds]
import fs from "node:fs";

const udd = process.argv[2];
const hold = Number(process.argv[3] || 5);
const [port, p] = fs.readFileSync(`${udd}/DevToolsActivePort`, "utf8").trim().split("\n");
const t0 = Date.now();
const ws = new WebSocket(`ws://127.0.0.1:${port}${p}`);
let next = 1;
const pending = new Map();
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
  }
};
ws.onclose = e => { console.log(`closed after ${Date.now() - t0} ms`, e.code, e.reason); process.exit(0); };
ws.onerror = e => console.log("ws error", e.message);
ws.onopen = async () => {
  console.log(`open after ${Date.now() - t0} ms`);
  const v = await call("Browser.getVersion");
  console.log(`first reply after ${Date.now() - t0} ms:`, v.product);
  const { targetInfos } = await call("Target.getTargets");
  console.log("targets:", targetInfos.map(t => `${t.type} ${t.url} ctx=${t.browserContextId?.slice(0, 8)}`).join("\n  "));
  const { browserContextIds } = await call("Target.getBrowserContexts");
  console.log("browser contexts (profiles other than the default show here?)", browserContextIds);
  const page = targetInfos.find(t => t.type === "page");
  if (page) {
    const { sessionId } = await call("Target.attachToTarget", { targetId: page.targetId, flatten: true });
    const r = await call("Runtime.evaluate", { expression: "location.href + ' ' + document.title", returnByValue: true }, sessionId);
    console.log("evaluated in a page:", r.result.value);
  }
  setTimeout(() => { ws.close(); }, hold * 1000);
};
