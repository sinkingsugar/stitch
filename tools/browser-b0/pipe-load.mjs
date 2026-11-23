// B0 spike: start a Chrome with --remote-debugging-pipe and load the
// extension with CDP Extensions.loadUnpacked (how tests can load it into
// branded Chrome, which ignores --load-extension since 137).
//   node pipe-load.mjs <chrome> <user-data-dir> <extension dir> [extra flags…]
import { spawn } from "node:child_process";
import fs from "node:fs";

const [exe, udd, ext, ...extra] = process.argv.slice(2);
fs.mkdirSync(udd + "/NativeMessagingHosts", { recursive: true });
const env = { ...process.env, DISPLAY: ":99" };
delete env.WAYLAND_DISPLAY;
const chrome = spawn(exe, ["--ozone-platform=x11", `--user-data-dir=${udd}`, "--no-first-run", "--no-default-browser-check",
  "--window-position=0,0", "--window-size=1400,950", "--remote-debugging-pipe", "--enable-unsafe-extension-debugging",
  ...extra, "about:blank"], { env, stdio: ["ignore", "ignore", "ignore", "pipe", "pipe"] });
const [toChrome, fromChrome] = [chrome.stdio[3], chrome.stdio[4]];
let buf = "";
const pending = new Map();
fromChrome.on("data", d => {
  buf += d;
  let i;
  while ((i = buf.indexOf("\0")) >= 0) {
    const msg = JSON.parse(buf.slice(0, i));
    buf = buf.slice(i + 1);
    if (msg.id && pending.has(msg.id)) pending.get(msg.id)(msg);
  }
});
let next = 1;
const call = (method, params = {}) => new Promise(r => {
  const id = next++;
  pending.set(id, r);
  toChrome.write(JSON.stringify({ id, method, params }) + "\0");
});
const v = await call("Browser.getVersion");
console.log("version", v.result?.product);
const r = await call("Extensions.loadUnpacked", { path: ext });
console.log("loadUnpacked", JSON.stringify(r));
// Keep Chrome (and its pipe) alive until killed.
process.on("SIGTERM", () => { chrome.kill(); process.exit(0); });
chrome.on("exit", c => { console.log("chrome exited", c); process.exit(0); });
