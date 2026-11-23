// B0 spike debugging aid: evaluate an expression in a target (the
// extension's service worker by default) over a browser's debugging port.
//   node cdp-eval.mjs <port> <url substring> <expression>
const [port, match, expression] = process.argv.slice(2);
const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const t = targets.find(t => t.url.includes(match));
if (!t) { console.log("no target; have:", targets.map(t => `${t.type} ${t.url}`).join("\n")); process.exit(1); }
const ws = new WebSocket(t.webSocketDebuggerUrl);
ws.onopen = () => ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression, awaitPromise: true, returnByValue: true } }));
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id === 1) { console.log(JSON.stringify(m.result?.result?.value ?? m.result ?? m.error)); process.exit(0); } };
