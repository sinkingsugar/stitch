// Stitch's extension: the service worker (docs/browser-tool.md §5.1).
//
// A thin relay between the local Stitch node (through the native messaging
// host `stitch browser-host`) and this Chrome profile: CDP per tab through
// chrome.debugger, tabs, tab groups and windows through their APIs, and
// what the user does (their input, Stop, Take over, Hand back). No model,
// no account, no server.
//
// What it enforces whatever the node says (§8.1):
// - nothing at all until the user enables Stitch in this profile (popup);
// - it attaches and relays only for tabs of a Stitch session: ones the
//   node opened here, or ones the user put in a session's group;
// - never chrome://, the Web Store, other extensions' pages or file://;
// - no cookies or storage APIs (it has no such permission);
// - Stop detaches at once, here, without the node.

const HOST = "com.fragcolor.stitch.browser";
const PROTO = 1;
// Stamped with the node's build id by `stitch browser install`; "dev" when
// loaded unpacked straight from the repository (never flagged as stale).
const BUILD = "dev";
// The debugger is let go after this long without a call on a tab, so
// Chrome's "started debugging this browser" bar can go (it stays ~5 s).
// A tab with a command in flight is never idle: detaching under it failed
// the command with "Detached while handling command." (issue w8dh).
const DETACH_IDLE_MS = 20000;
const IDLE_CHECK_MS = 5000;
// Chrome draws nothing for a tab whose window is hidden or covered (a
// full-screen game, a minimised window), and Page.captureScreenshot then
// waits for a frame that does not come: given up after this long, under
// the node's 30 s for a call, with a reason.
const SHOT_TIMEOUT_MS = 25000;

let port = null;
let backoff = 500;

// Sessions and the tabs they own. Kept in chrome.storage.session so a
// restarted service worker still knows them (tab IDs live as long as
// Chrome does).
//   sessions: { [id]: { title, color, mode, window, group, stopped } }
//   tabs:     { [tabId]: { session, held, shared } }
//   offered:  { [tabId]: true }: tabs the user shared or pointed at with no
//             session yet, which a session may then take (adopt)
let state = { sessions: {}, tabs: {}, offered: {}, downloads: {} };
const attached = new Map(); // tabId -> last use (ms)
const busy = new Map(); // tabId -> CDP commands in flight
const loaded = (async () => {
  const s = await chrome.storage.session.get("state");
  if (s.state) state = s.state;
  state.offered ??= {};
})();
async function save() {
  await chrome.storage.session.set({ state });
}

// The profile id is created once per service worker (issue dcez): two
// concurrent settings() calls (startup connect and popup enable) used to
// both find no id and store two different ones.
let idOnce = null;
function profileId() {
  return (idOnce ??= (async () => {
    const s = await chrome.storage.local.get("id");
    if (s.id) return s.id;
    const id = crypto.randomUUID();
    await chrome.storage.local.set({ id });
    return id;
  })().catch((e) => {
    idOnce = null;
    throw e;
  }));
}

async function settings() {
  const [id, s] = await Promise.all([profileId(), chrome.storage.local.get(["name", "enabled"])]);
  return { id, name: s.name || "", enabled: !!s.enabled };
}

function send(msg) {
  if (!port) return;
  try {
    port.postMessage(msg);
  } catch (e) {
    console.warn("stitch: post failed", e);
  }
}

function event(kind, fields) {
  send({ t: "event", kind, at: Date.now(), ...fields });
}

async function hello() {
  // The sessions listed are the stored ones: the node reopens any not listed.
  await loaded;
  const profile = await settings();
  const brand = navigator.userAgentData?.brands?.map(b => `${b.brand} ${b.version}`).join(", ") || "";
  send({
    t: "hello",
    proto: PROTO,
    build: BUILD,
    version: chrome.runtime.getManifest().version,
    profile,
    browser: { ua: navigator.userAgent, brand },
    sessions: Object.keys(state.sessions),
    tabs: Object.keys(state.tabs).map(Number),
  });
}

function connect() {
  if (port) return;
  try {
    port = chrome.runtime.connectNative(HOST);
  } catch (e) {
    port = null;
    setTimeout(connect, backoff);
    backoff = Math.min(backoff * 2, 30000);
    return;
  }
  port.onMessage.addListener(onMessage);
  port.onDisconnect.addListener(() => {
    const why = chrome.runtime.lastError?.message;
    if (why) console.warn("stitch: host gone:", why);
    port = null;
    setTimeout(connect, backoff);
    backoff = Math.min(backoff * 2, 30000);
  });
  hello().then(() => (backoff = 500));
}

chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);

// "Ask Stitch about this…" on a page, a selection, a link, an image or a
// field: what the user points at goes to the node (browser_context).
function menus() {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: "stitch-ask", title: "Ask Stitch about this…",
      contexts: ["page", "selection", "link", "image", "editable"] });
  });
}
chrome.runtime.onInstalled.addListener(menus);
chrome.runtime.onStartup.addListener(menus);
chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== "stitch-ask" || !tab) return;
  await loaded;
  if (!(await settings()).enabled || forbiddenUrl(tab.url)) return;
  const el = await toPage(tab.id, { t: "pointed" }).catch(() => null);
  if (!state.tabs[tab.id]) state.offered[tab.id] = true;
  await save();
  event("point", { tab: tab.id, url: tab.url, title: tab.title, how: "menu",
    selection: info.selectionText || "", link: info.linkUrl || null, src: info.srcUrl || null,
    element: el?.result?.element || null, ref: el?.result?.ref || null, doc: el?.result?.doc || null,
    session: state.tabs[tab.id]?.session || null });
});
connect();

// --- what the extension refuses ------------------------------------------

function forbiddenUrl(url) {
  if (!url) return null;
  if (/^(chrome|chrome-untrusted|devtools|edge|brave|about):/i.test(url) && url !== "about:blank") return "a browser page";
  if (/^chrome-extension:/i.test(url)) return "an extension's page";
  if (/^file:/i.test(url)) return "a local file";
  if (/^https:\/\/(chromewebstore\.google\.com|chrome\.google\.com\/webstore)/i.test(url)) return "the Chrome Web Store";
  return null;
}

async function ownedTab(tabId) {
  await loaded;
  const t = state.tabs[tabId];
  if (!t) throw new Error(`tab ${tabId} is not one of Stitch's tabs`);
  const s = state.sessions[t.session];
  if (!s || s.stopped) throw new Error("the user stopped this session");
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab) throw new Error(`tab ${tabId} is closed`);
  const bad = forbiddenUrl(tab.url || tab.pendingUrl);
  if (bad) throw new Error(`Stitch does not work in ${bad}`);
  return { tab, own: t, session: s };
}

// A message to a page's script. A tab loaded before the extension was
// (installed, reloaded or updated) has none: it is injected then, once.
// A tab between documents (a link just followed) has no script either, and
// Chrome refuses the injection there ("Cannot access contents of the page.
// Extension manifest must request permission…", whatever the manifest
// says): the new document gets its script by itself at document_start, so
// that is waited for, up to PAGE_WAIT_MS (issue y53x).
const PAGE_WAIT_MS = 8000;
const NO_SCRIPT = /Receiving end does not exist|Could not establish connection/;
const BETWEEN = /Cannot access contents of the page|Frame with ID \d+ (was removed|is showing error page)|No frame with id/i;

async function toPage(tabId, msg, frameId = 0) {
  const ask = () => chrome.tabs.sendMessage(tabId, msg, { frameId });
  try {
    return await ask();
  } catch (e) {
    if (!NO_SCRIPT.test(String(e?.message || e))) throw e;
  }
  try {
    await chrome.scripting.executeScript({ target: { tabId, allFrames: true }, files: ["content.js"] });
    return await ask();
  } catch (e) {
    const why = String(e?.message || e);
    if (!BETWEEN.test(why)) throw e;
    const end = Date.now() + PAGE_WAIT_MS;
    for (;;) {
      await new Promise(ok => setTimeout(ok, 150));
      const tab = await chrome.tabs.get(tabId).catch(() => null);
      if (!tab) throw new Error(`tab ${tabId} is closed`);
      try {
        return await ask();
      } catch (e2) {
        if (!NO_SCRIPT.test(String(e2?.message || e2))) throw e2;
      }
      if (Date.now() > end) {
        throw new Error(`the page cannot be read: ${tab.status === "loading" ? "it is still loading" : "Chrome shows no page Stitch may read in it (an error page, or the tab crashed: reload it)"} (${why})`);
      }
    }
  }
}

async function enabled() {
  if (!(await settings()).enabled) {
    throw new Error("Stitch is not enabled in this Chrome profile: open the Stitch extension there and switch it on");
  }
}

// --- debugger ---------------------------------------------------------------

// Chrome refuses chrome.debugger on a tab that holds a frame of another
// extension ("Cannot access a chrome-extension:// URL of different
// extension"): password managers and privacy extensions inject such
// frames (autofill dropdowns, notifications) into pages, often inside
// closed shadow roots, once they see login or email fields. Then the
// attach finds them, removes them and tries once more; when that is still
// refused, input and screenshots fall back to the page's script (below).
const FOREIGN = /different extension|chrome-extension:\/\//i;
// Extensions known to inject such frames, for the message (an extension's
// name is not readable without the management permission).
const KNOWN_EXTENSIONS = {
  ghmbeldphafepmbegfdlkpapadhbakde: "Proton Pass",
  bkdgflcldnnnapblkhphbgpggdiikppg: "DuckDuckGo Privacy Essentials",
  nngceckbapebfimnlniiiahkandclblb: "Bitwarden",
  aeblfdkhhhdcdjpifhhbdiojplfjncoa: "1Password",
  hdokiejnpimakedhajhdlcegeplioahd: "LastPass",
  fdjamakpfbbddfjaooikfcpapjohcfmg: "Dashlane",
  kbfnbcaeplbcioakkpcpgfkobkghlhen: "Grammarly",
};
// Tabs whose attach was refused for such a frame: { until, why }. Calls
// in this window go straight to the fallback (one click is several CDP
// calls, which must not be split between the two); a navigation ends it.
const REFUSED_MS = 15000;
const refused = new Map();

class Blocked extends Error {}

// The frames of other extensions in a tab, in every frame of it and
// inside shadow roots (open, and closed through chrome.dom); removed when
// `remove`. Runs in the page (an isolated world), so it is self-contained.
function foreignFramesInPage(remove, own) {
  const found = [];
  const shadowOf = el => {
    try {
      return chrome.dom?.openOrClosedShadowRoot?.(el) || el.shadowRoot || null;
    } catch {
      return el.shadowRoot || null;
    }
  };
  const visit = root => {
    for (const el of root.querySelectorAll("*")) {
      if (/^(IFRAME|FRAME|EMBED|OBJECT)$/.test(el.tagName)) {
        const src = String(el.src || el.data || el.getAttribute("src") || el.getAttribute("data") || "");
        const m = /^chrome-extension:\/\/([a-p]{32})/i.exec(src);
        if (m && m[1] !== own) {
          let host = el.getRootNode()?.host || null;
          while (host?.getRootNode()?.host) host = host.getRootNode().host;
          found.push({ id: m[1], src: src.slice(0, 120), host: host ? host.localName : null });
          if (remove) el.remove();
        }
      }
      const sr = shadowOf(el);
      if (sr) visit(sr);
    }
  };
  if (document.documentElement) visit(document);
  return found;
}

async function foreignFrames(tabId, remove) {
  const runs = await chrome.scripting.executeScript({
    target: { tabId, allFrames: true },
    func: foreignFramesInPage,
    args: [!!remove, chrome.runtime.id],
  }).catch(() => []);
  const all = runs.flatMap(r => r.result || []);
  const seen = new Map();
  for (const f of all) if (!seen.has(f.id)) seen.set(f.id, f);
  return [...seen.values()];
}

function blockedMessage(frames, removed) {
  const who = frames.length
    ? frames.map(f => `extension ${f.id}${KNOWN_EXTENSIONS[f.id] ? ` (${KNOWN_EXTENSIONS[f.id]})` : ""}${f.host ? ` in <${f.host}>` : ""}`).join(", ")
    : "another extension (its frame was not found in the page)";
  return `another extension's frame blocks control of this tab: ${who}${removed ? ", which put it back after it was removed" : ""}. ` +
    "Chrome lets no extension debug a tab holding another extension's frame. Ask the user to turn that extension off for this site " +
    "(its site access: \"On click\", or its own setting for autofill), or to reload the tab without it.";
}

async function rawAttach(tabId) {
  try {
    await chrome.debugger.attach({ tabId }, "1.3");
  } catch (e) {
    if (!/already attached/i.test(String(e?.message))) throw e;
  }
}

async function attach(tabId) {
  if (attached.has(tabId)) {
    attached.set(tabId, Date.now());
    return;
  }
  const r = refused.get(tabId);
  if (r && Date.now() < r.until) throw new Blocked(r.why);
  refused.delete(tabId);
  try {
    await rawAttach(tabId);
  } catch (e) {
    const why = String(e?.message || e);
    if (!FOREIGN.test(why)) throw new Error(`could not attach to the tab (is DevTools open on it?): ${why}`);
    // Another extension's frame: removed, then once more.
    const frames = await foreignFrames(tabId, true);
    let again = null;
    if (frames.length) {
      await new Promise(ok => setTimeout(ok, 100));
      again = await rawAttach(tabId).then(() => null, e2 => e2);
    }
    if (!frames.length || again) {
      if (again && !FOREIGN.test(String(again?.message || again))) {
        throw new Error(`could not attach to the tab: ${again?.message || again}`);
      }
      const left = frames.length ? await foreignFrames(tabId, false) : [];
      const msg = blockedMessage(left.length ? left : frames, frames.length > 0 && left.length > 0);
      refused.set(tabId, { until: Date.now() + REFUSED_MS, why: msg });
      throw new Blocked(msg);
    }
    event("foreign_frames_removed", { tab: tabId, frames });
  }
  attached.set(tabId, Date.now());
  // Dialogs and downloads are told as Page events.
  await chrome.debugger.sendCommand({ tabId }, "Page.enable", {}).catch(() => {});
  // A tab behind others behaves as focused (B0: fresh frames, input lands).
  await chrome.debugger.sendCommand({ tabId }, "Emulation.setFocusEmulationEnabled", { enabled: true }).catch(() => {});
}

async function detach(tabId) {
  if (!attached.has(tabId)) return;
  attached.delete(tabId);
  await chrome.debugger.detach({ tabId }).catch(() => {});
}

// A CDP command on a session tab. Detached under it (another extension's
// frame came in and Chrome let go): attached again once. Refused for
// another extension's frame: the fallback, or the reason.
// The tab counts as busy meanwhile, so the idle timer leaves it attached.
async function debuggerCall(tab, method, params) {
  busy.set(tab, (busy.get(tab) || 0) + 1);
  try {
    return await debuggerTry(tab, method, params);
  } finally {
    const n = (busy.get(tab) || 1) - 1;
    if (n > 0) busy.set(tab, n);
    else busy.delete(tab);
    if (attached.has(tab)) attached.set(tab, Date.now());
  }
}

// A screenshot that does not come in SHOT_TIMEOUT_MS: the tab is let go
// (dropping the command Chrome still waits on) and the reason given.
async function command(tab, method, params) {
  const sent = chrome.debugger.sendCommand({ tabId: tab }, method, params);
  if (method !== "Page.captureScreenshot") return await sent;
  let timer;
  const late = new Promise(ok => (timer = setTimeout(() => ok(true), SHOT_TIMEOUT_MS)));
  sent.catch(() => {});
  try {
    const first = await Promise.race([sent.then(r => ({ r })), late]);
    if (first !== true) return first.r;
  } finally {
    clearTimeout(timer);
  }
  await detach(tab);
  throw new Error(`Chrome made no screenshot of this tab in ${Math.round(SHOT_TIMEOUT_MS / 1000)} s: it draws nothing for a tab ` +
    "whose window is hidden or covered (minimised, or under a full-screen app or game). Read the page with browser_snapshot, " +
    "or ask the user to show the window");
}

async function debuggerTry(tab, method, params) {
  for (let tries = 0; ; tries++) {
    try {
      await attach(tab);
    } catch (e) {
      if (e instanceof Blocked) return await fallback(tab, method, params, e.message);
      throw e;
    }
    try {
      return await command(tab, method, params);
    } catch (e) {
      const why = String(e?.message || e);
      if (tries === 0 && /not attached|detached|Cannot access a chrome-extension/i.test(why)) {
        attached.delete(tab);
        continue;
      }
      throw e;
    }
  }
}

// --- without the debugger ---------------------------------------------------
// When Chrome refuses the debugger (another extension's frame), input goes
// through the page's script as synthetic DOM events: pages see them as
// untrusted (event.isTrusted false), and some ignore those. Screenshots
// come from chrome.tabs.captureVisibleTab, which needs the tab shown in
// its window. Results carry `stitch_fallback`, which the node passes on.
const SYNTHETIC = "the debugger is blocked here, so this was done with synthetic DOM events from the page's script: " +
  "pages see them as untrusted (isTrusted false) and some ignore them; check the result";

async function fallback(tab, method, params, why) {
  switch (method) {
    case "Input.dispatchMouseEvent":
    case "Input.dispatchKeyEvent":
    case "Input.insertText": {
      if (state.tabs[tab]?.held) throw new Error("the user has taken over this tab");
      const r = await toPage(tab, { t: "synth", method, params });
      if (!r?.ok) throw new Error(`${r?.error || "the page's script did not answer"} (${why})`);
      return { ...(r.result || {}), stitch_fallback: `${SYNTHETIC} (${why})` };
    }
    case "Page.getLayoutMetrics": {
      const r = await toPage(tab, { t: "metrics" });
      if (!r?.ok) throw new Error(why);
      return { ...r.result, stitch_fallback: why };
    }
    case "Page.captureScreenshot":
      return await captureVisible(tab, params, why);
    case "Page.navigate":
      await chrome.tabs.update(tab, { url: params.url });
      return { stitch_fallback: why };
    case "Page.reload":
      await chrome.tabs.reload(tab);
      return { stitch_fallback: why };
    default:
      throw new Error(`${why} (${method} needs the debugger)`);
  }
}

// The tab's visible area, cut to the clip (page CSS pixels) and scaled as
// CDP's Page.captureScreenshot would.
async function captureVisible(tab, params, why) {
  const t = await chrome.tabs.get(tab);
  const w = await chrome.windows.get(t.windowId).catch(() => null);
  if (!t.active || !w || w.state === "minimized") {
    throw new Error(`${why} Without the debugger a screenshot needs the tab shown in its window (it is ${w?.state === "minimized" ? "in a minimised window" : "behind other tabs"}): ` +
      "ask the user to bring it to the front, or read the page with browser_snapshot");
  }
  const info = await toPage(tab, { t: "info" });
  if (!info?.ok) throw new Error(why);
  const { scroll, viewport } = info.result;
  const url = await chrome.tabs.captureVisibleTab(t.windowId, { format: "png" });
  const bmp = await createImageBitmap(await (await fetch(url)).blob());
  const px = bmp.width / viewport.w; // device pixels per CSS pixel
  const clip = params.clip || { x: scroll.x, y: scroll.y, width: viewport.w, height: viewport.h, scale: 1 };
  const scale = clip.scale || 1;
  // The clip in viewport CSS pixels; what lies beyond the viewport is not there.
  const x0 = Math.max(clip.x - scroll.x, 0), y0 = Math.max(clip.y - scroll.y, 0);
  const x1 = Math.min(clip.x - scroll.x + clip.width, viewport.w), y1 = Math.min(clip.y - scroll.y + clip.height, viewport.h);
  if (x1 <= x0 || y1 <= y0) throw new Error(`${why} Without the debugger only the visible viewport can be captured, and the area is outside it`);
  const ow = Math.max(1, Math.round(clip.width * scale)), oh = Math.max(1, Math.round(clip.height * scale));
  const canvas = new OffscreenCanvas(ow, oh);
  const g = canvas.getContext("2d");
  g.drawImage(bmp, x0 * px, y0 * px, (x1 - x0) * px, (y1 - y0) * px,
    (x0 - (clip.x - scroll.x)) * scale, (y0 - (clip.y - scroll.y)) * scale, (x1 - x0) * scale, (y1 - y0) * scale);
  const blob = await canvas.convertToBlob({ type: "image/png" });
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  const partial = x0 > clip.x - scroll.x || y0 > clip.y - scroll.y || x1 < clip.x - scroll.x + clip.width || y1 < clip.y - scroll.y + clip.height;
  return { data: btoa(bin), stitch_fallback: `the debugger is blocked here, so this is a capture of the visible tab${partial ? " (only the part of the area inside the viewport; the rest is blank)" : ""} (${why})` };
}

setInterval(() => {
  const now = Date.now();
  for (const [tabId, used] of attached) if (!busy.has(tabId) && now - used > DETACH_IDLE_MS) detach(tabId);
}, IDLE_CHECK_MS);

chrome.debugger.onDetach.addListener((src, reason) => {
  if (src.tabId == null) return;
  attached.delete(src.tabId);
  // Chrome also detaches when another extension's frame comes into the
  // tab (target_closed while the tab lives on): the next call attaches
  // again, removing it, or falls back (debuggerCall).
  // Cancel or × on Chrome's bar detaches every tab: the user's Stop (B0).
  if (reason === "canceled_by_user") stopAll("the user cancelled debugging in Chrome's bar");
  else event("detached", { tab: src.tabId, reason });
});

// Downloads a session tab starts: matched by URL to the tab's
// Page.downloadWillBegin; executables and archives are paused until the
// user decides (browser_downloads keep asks them, §8.3).
const RISKY = /\.(exe|msi|dmg|pkg|app|sh|bat|cmd|ps1|deb|rpm|apk|jar|zip|tar|gz|tgz|bz2|xz|7z|rar|iso)$/i;
const begun = []; // { tab, url, at }
const acted = new Map(); // tab -> when the agent last did something there
chrome.downloads.onCreated.addListener(async item => {
  await loaded;
  const now = Date.now();
  while (begun.length && now - begun[0].at > 30000) begun.shift();
  const same = (a, b) => a === b || (a && b && a.replace(/#.*/, "") === b.replace(/#.*/, ""));
  const i = begun.findIndex(b => same(b.url, item.url) || same(b.url, item.finalUrl));
  let tab = i >= 0 ? begun.splice(i, 1)[0].tab : null;
  // Else by the page it came from (the referrer), among session tabs:
  // exactly, then by origin (Chrome trims referrers).
  if (tab == null && item.referrer) {
    const origin = u => { try { return new URL(u).origin; } catch { return null; } };
    const ro = origin(item.referrer);
    let byOrigin = null;
    for (const id of Object.keys(state.tabs)) {
      const t = await chrome.tabs.get(Number(id)).catch(() => null);
      if (t?.url === item.referrer) {
        tab = t.id;
        break;
      }
      if (byOrigin == null && ro && ro !== "null" && origin(t?.url) === ro) byOrigin = t.id;
    }
    tab ??= byOrigin;
  }
  // Else the session tab the agent acted on a moment ago (a click that
  // starts a blob: download has neither URL nor referrer to match).
  if (tab == null) {
    let best = 0;
    for (const [id, at] of acted) {
      if (state.tabs[id] && now - at < 10000 && at > best) [tab, best] = [id, at];
    }
  }
  if (tab == null) return;
  const own = state.tabs[tab];
  if (!own) return;
  const name = item.filename || item.url.split(/[?#]/)[0].split("/").pop() || "";
  const risky = RISKY.test(name) || RISKY.test(item.url.split(/[?#]/)[0]);
  if (risky) await chrome.downloads.pause(item.id).catch(() => {});
  state.downloads ??= {};
  state.downloads[item.id] = { tab, session: own.session };
  await save();
  event("download", { tab, session: own.session, id: item.id, url: item.url, risky });
});

chrome.debugger.onEvent.addListener((src, method, params) => {
  if (method === "Page.downloadWillBegin" && state.tabs[src.tabId]) begun.push({ tab: src.tabId, url: params.url, at: Date.now() });
  if (src.tabId == null || !state.tabs[src.tabId]) return;
  if (method === "Page.javascriptDialogOpening" || method === "Page.frameNavigated" && !params.frame?.parentId
    || method === "Page.downloadWillBegin" || method === "Page.fileChooserOpened") {
    event("cdp", { tab: src.tabId, method, params });
  }
});

// --- sessions, tabs, groups, windows -------------------------------------------

async function userWindow() {
  const w = await chrome.windows.getLastFocused({ windowTypes: ["normal"] }).catch(() => null);
  if (w && !Object.values(state.sessions).some(s => s.mode === "alone" && s.window === w.id)) return w.id;
  const all = await chrome.windows.getAll({ windowTypes: ["normal"] });
  const mine = new Set(Object.values(state.sessions).filter(s => s.mode === "alone").map(s => s.window));
  const other = all.find(x => !mine.has(x.id));
  if (other) return other.id;
  const made = await chrome.windows.create({ focused: false, state: "normal" });
  return made.id;
}

// The agent's own window: a fixed size (a window manager resizing it moved
// the viewport under coordinate clicks) and, asked not to take focus. Some
// window managers ignore focused:false; if Chrome had the user's focus and
// the new window took it, it goes back to the previous window, once, right
// after the creation. If Chrome was not focused (the user is in another
// app), nothing is refocused: that would bring Chrome to the front.
async function createAloneWindow(url) {
  const prev = await chrome.windows.getLastFocused({ windowTypes: ["normal"] }).catch(() => null);
  const opts = { focused: false, state: "normal", width: 1280, height: 900 };
  if (url) opts.url = url;
  const made = await chrome.windows.create(opts);
  if (prev?.id != null && prev.id !== made.id && prev.focused === true) {
    const now = await chrome.windows.get(made.id).catch(() => null);
    if (now?.focused || made.focused) await chrome.windows.update(prev.id, { focused: true }).catch(() => {});
  }
  return made;
}

async function groupGone(s) {
  if (s.group == null) return true;
  return !(await chrome.tabGroups.get(s.group).catch(() => null));
}

// Tabs being grouped or moved by the extension itself: their group
// changes are not the user's.
const grouping = new Set();

async function intoGroup(sessionId, tabId) {
  const s = state.sessions[sessionId];
  grouping.add(tabId);
  try {
    if (!(await groupGone(s))) {
      await chrome.tabs.group({ tabIds: [tabId], groupId: s.group });
    } else {
      // A new group in the tab's own window. Without a window Chrome makes
      // it in the "current" one, for a service worker the last activated
      // window of any type: a popup, an installed app's or DevTools' window
      // refuses ("Grouping is not supported by tabs in this window"), and a
      // normal one gets the tab moved into it, out of the session's own
      // window (issue y53x).
      const t = await chrome.tabs.get(tabId);
      s.group = await chrome.tabs.group({ tabIds: [tabId], createProperties: { windowId: t.windowId } });
    }
    await chrome.tabGroups.update(s.group, { title: s.title, color: s.color || "purple" });
  } finally {
    setTimeout(() => grouping.delete(tabId), 500);
  }
}

// After a group moved between windows: its ID as its tabs now say.
async function regroup(sessionId, owned) {
  const s = state.sessions[sessionId];
  const first = owned.length ? await chrome.tabs.get(owned[0]).catch(() => null) : null;
  if (first && first.groupId !== -1) s.group = first.groupId;
  setTimeout(() => owned.forEach(id => grouping.delete(id)), 500);
}

const ops = {
  async open({ session, title, color, mode }) {
    await loaded;
    const s = state.sessions[session] || { group: null, window: null };
    const wasStopped = !!s.stopped;
    Object.assign(s, { title: title || "Stitch", color: color || "purple", mode: mode || "coop", stopped: false });
    state.sessions[session] = s;
    await save();
    // Renamed (the thread got its title): the group says so.
    if (!(await groupGone(s))) await chrome.tabGroups.update(s.group, { title: s.title, color: s.color }).catch(() => {});
    // Going on after a Stop: its tabs' overlays come back.
    if (wasStopped) {
      for (const [id, t] of Object.entries(state.tabs)) {
        if (t.session !== session) continue;
        chrome.tabs.sendMessage(Number(id), { t: "overlay", stopped: false, held: t.held, state: s.title }, { frameId: 0 }).catch(() => {});
      }
    }
    return {};
  },

  async newTab({ session, url }) {
    await loaded;
    const s = state.sessions[session];
    if (!s) throw new Error(`no session ${session}`);
    if (s.stopped) throw new Error("the user stopped this session");
    const bad = forbiddenUrl(url);
    if (bad) throw new Error(`Stitch does not work in ${bad}`);
    let tab;
    if (s.mode === "alone") {
      const w = s.window != null && (await chrome.windows.get(s.window).catch(() => null));
      if (w && (!w.type || w.type === "normal")) {
        tab = await chrome.tabs.create({ windowId: w.id, url, active: true });
      } else {
        // Its own window, never focused: the agent does not take the user's.
        const made = await createAloneWindow(url);
        s.window = made.id;
        tab = made.tabs[0];
        s.group = null;
      }
    } else {
      // In the user's window, in the background: nothing is activated.
      tab = await chrome.tabs.create({ windowId: await userWindow(), url, active: false });
    }
    // The tab is open now: it is the session's whatever its group does, and
    // saved, so a failed grouping leaves no tab half-registered.
    state.tabs[tab.id] = { session, held: false, shared: false };
    let note = null;
    try {
      await intoGroup(session, tab.id);
    } catch (e) {
      note = String(e?.message || e);
      // An installed web app may capture the URL into its own window, which
      // takes no groups: the tab goes back to a normal window, grouped there.
      const t = await chrome.tabs.get(tab.id).catch(() => null);
      const w = t && (await chrome.windows.get(t.windowId).catch(() => null));
      if (t && w && w.type && w.type !== "normal") {
        const target = s.mode === "alone" && s.window != null && s.window !== t.windowId ? s.window : await userWindow();
        grouping.add(tab.id);
        const moved = await chrome.tabs.move(tab.id, { windowId: target, index: -1 }).then(() => true, () => false);
        if (moved && (await intoGroup(session, tab.id).then(() => true, () => false))) note = null;
      }
    }
    await save();
    tab = (await chrome.tabs.get(tab.id).catch(() => null)) || tab;
    return { tab: tab.id, window: tab.windowId, group: s.group,
      ...(note ? { note: `the tab is open but not in the session's group (${note})` } : {}) };
  },

  async closeTab({ tab }) {
    await ownedTab(tab);
    await detach(tab);
    delete state.tabs[tab];
    await save();
    await chrome.tabs.remove(tab).catch(() => {});
    return {};
  },

  async tabs({ session }) {
    await loaded;
    const out = [];
    for (const [id, t] of Object.entries(state.tabs)) {
      if (session && t.session !== session) continue;
      const tab = await chrome.tabs.get(Number(id)).catch(() => null);
      if (!tab) {
        delete state.tabs[id];
        continue;
      }
      out.push({ tab: tab.id, session: t.session, url: tab.url || tab.pendingUrl, title: tab.title, active: tab.active,
        window: tab.windowId, held: t.held, shared: t.shared, loading: tab.status === "loading" });
    }
    await save();
    return out;
  },

  async cdp({ tab, method, params }) {
    await enabled();
    await ownedTab(tab);
    if (/^(Browser|Target|SystemInfo|Storage|Network\.getAllCookies|Network\.getCookies)/.test(method)) {
      throw new Error(`${method} is not relayed`);
    }
    if (/^Input\./.test(method)) acted.set(tab, Date.now());
    return await debuggerCall(tab, method, params || {});
  },

  async page({ tab, msg }) {
    await enabled();
    const { own } = await ownedTab(tab);
    if (own.held && msg.t !== "overlay" && msg.t !== "ask") throw new Error("the user has taken over this tab");
    if (msg.t === "fill" || msg.t === "synth") acted.set(tab, Date.now()); // acts, not reads
    return await toPage(tab, msg);
  },

  async detach({ tab }) {
    await detach(tab);
    return {};
  },

  // Cooperative (the user's window) or agent-alone (a window of its own).
  async mode({ session, mode }) {
    await enabled();
    await loaded;
    const s = state.sessions[session];
    if (!s) throw new Error(`no session ${session}`);
    if (s.mode === mode) return { mode };
    const owned = Object.entries(state.tabs).filter(([, t]) => t.session === session).map(([id]) => Number(id));
    owned.forEach(id => grouping.add(id));
    if (mode === "alone") {
      const made = await createAloneWindow();
      const blank = made.tabs?.[0]?.id;
      if (!(await groupGone(s))) await chrome.tabGroups.move(s.group, { windowId: made.id, index: -1 });
      else if (owned.length) await chrome.tabs.move(owned, { windowId: made.id, index: -1 });
      if (blank != null && owned.length) await chrome.tabs.remove(blank).catch(() => {});
      s.window = made.id;
    } else {
      const target = await userWindow();
      const old = s.window;
      if (!(await groupGone(s))) await chrome.tabGroups.move(s.group, { windowId: target, index: -1 });
      s.window = null;
      if (old != null) {
        const left = await chrome.tabs.query({ windowId: old }).catch(() => []);
        if (!left.length) await chrome.windows.remove(old).catch(() => {});
      }
    }
    s.mode = mode;
    await regroup(session, owned);
    await save();
    return { mode };
  },

  // The end of a session: its tabs stay open for the user, ungrouped.
  async end({ session }) {
    await loaded;
    const ids = Object.entries(state.tabs).filter(([, t]) => t.session === session).map(([id]) => Number(id));
    for (const id of ids) {
      await detach(id);
      await chrome.tabs.sendMessage(id, { t: "overlay", off: true }, { frameId: 0 }).catch(() => {});
      delete state.tabs[id];
    }
    if (ids.length) await chrome.tabs.ungroup(ids).catch(() => {});
    delete state.sessions[session];
    await save();
    return {};
  },

  // A tab the user shared or pointed at, taken by a session (and only
  // such a tab: the user offered it).
  async adopt({ session, tab }) {
    await loaded;
    const s = state.sessions[session];
    if (!s) throw new Error(`no session ${session}`);
    if (!state.offered[tab] && !state.tabs[tab]) throw new Error("the user has not shared that tab");
    const t = await chrome.tabs.get(tab).catch(() => null);
    if (!t) throw new Error(`tab ${tab} is closed`);
    const bad = forbiddenUrl(t.url);
    if (bad) throw new Error(`Stitch does not work in ${bad}`);
    delete state.offered[tab];
    state.tabs[tab] = { session, held: false, shared: true };
    await intoGroup(session, tab);
    await save();
    toPage(tab, { t: "overlay", state: s.title }).catch(() => {});
    return { tab, url: t.url, title: t.title };
  },

  // The session's downloads, with where they are and how far.
  async downloads({ session }) {
    await loaded;
    const mine = Object.entries(state.downloads || {}).filter(([, d]) => d.session === session).map(([id]) => Number(id));
    if (!mine.length) return [];
    const items = await chrome.downloads.search({});
    return items.filter(d => mine.includes(d.id)).map(d => ({ id: d.id, url: d.url, path: d.filename, state: d.state,
      paused: d.paused, bytes: d.bytesReceived, total: d.totalBytes, mime: d.mime, danger: d.danger, error: d.error || null }));
  },

  async download({ session, id, keep }) {
    await loaded;
    const d = (state.downloads || {})[id];
    if (!d || d.session !== session) throw new Error(`download ${id} is not this session's`);
    if (keep) await chrome.downloads.resume(id);
    else await chrome.downloads.cancel(id);
    return {};
  },

  // `stitch browser profiles disable`: switching off is safe either way, so
  // the node may do it (switching on takes a click in the popup).
  async disable() {
    await chrome.storage.local.set({ enabled: false });
    await stopAll("Stitch was switched off for this profile from the node");
    await hello();
    return {};
  },

  // The agent hands a tab to the user (browser_handoff): held until Hand
  // back, as Take over does.
  async hold({ tab, reason }) {
    const { own, session } = await ownedTab(tab);
    own.held = true;
    await save();
    await detach(tab);
    if (session.group != null) await chrome.tabGroups.update(session.group, { title: `${session.title} · needs you` }).catch(() => {});
    await toPage(tab, { t: "overlay", held: true, state: `Your turn: ${reason}. Hand back when done.` }).catch(() => {});
    return {};
  },

  // A handoff ended outside the page (the user answered its question in
  // Stitch): the tab is the agent's again, as Hand back does.
  async release({ tab }) {
    const { own, session } = await ownedTab(tab);
    if (!own.held) return {};
    own.held = false;
    await save();
    if (session.group != null) await chrome.tabGroups.update(session.group, { title: session.title }).catch(() => {});
    await toPage(tab, { t: "overlay", held: false, state: session.title }).catch(() => {});
    return {};
  },

  // What "this" means: the user's active tab (URL and title only) and the
  // session tabs' states.
  async context() {
    await loaded;
    const w = await chrome.windows.getLastFocused({ windowTypes: ["normal"] }).catch(() => null);
    const [active] = w ? await chrome.tabs.query({ active: true, windowId: w.id }) : [];
    const idle = await chrome.idle.queryState(60);
    return {
      profile: await settings(),
      // Whose tab it is: a Stitch tab may be another session's (issue y53x).
      active: active ? { tab: active.id, url: active.url, title: active.title, stitch: !!state.tabs[active.id],
        session: state.tabs[active.id]?.session || null } : null,
      focused: w?.focused ?? false,
      idle,
    };
  },
};

// The node found this extension older than its files: reload (an unpacked
// extension re-reads them), at most once per 10 minutes, never when the
// builds agree or this one is "dev".
async function maybeReload(build) {
  if (!build || build === BUILD || BUILD === "dev") return;
  const { reloadedAt = 0 } = await chrome.storage.local.get("reloadedAt");
  if (Date.now() - reloadedAt < 10 * 60 * 1000) return;
  await chrome.storage.local.set({ reloadedAt: Date.now() });
  chrome.runtime.reload();
}

async function onMessage(msg) {
  if (msg.t === "reload") return maybeReload(msg.build);
  if (msg.t !== "req") return;
  try {
    if (!["context", "end", "detach", "disable"].includes(msg.op)) await enabled();
    const fn = ops[msg.op];
    if (!fn) throw new Error(`unknown op ${msg.op}`);
    const result = await fn(msg);
    send({ t: "res", id: msg.id, ok: true, result: result ?? {} });
  } catch (e) {
    send({ t: "res", id: msg.id, ok: false, error: String(e?.message || e) });
  }
}

// --- the user ------------------------------------------------------------------

async function stopSession(sessionId, why) {
  await loaded;
  const s = state.sessions[sessionId];
  if (!s) return;
  s.stopped = true;
  for (const [id, t] of Object.entries(state.tabs)) {
    if (t.session !== sessionId) continue;
    await detach(Number(id));
    chrome.tabs.sendMessage(Number(id), { t: "overlay", state: "Stopped", stopped: true }, { frameId: 0 }).catch(() => {});
  }
  await save();
  event("stop", { session: sessionId, why });
}

async function stopAll(why) {
  await loaded;
  for (const id of Object.keys(state.sessions)) await stopSession(id, why);
}

chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  (async () => {
    await loaded;
    // From the popup.
    if (!sender.tab) {
      if (msg.t === "popup") return reply(await popupState());
      if (msg.t === "enable") {
        await chrome.storage.local.set({ enabled: !!msg.on, ...(msg.name != null ? { name: msg.name } : {}) });
        if (!msg.on) await stopAll("Stitch was switched off in this profile");
        await hello();
        return reply(await popupState());
      }
      if (msg.t === "stopSession") {
        await stopSession(msg.session, "stopped from the popup");
        return reply(await popupState());
      }
      if (msg.t === "mode") {
        try { await ops.mode({ session: msg.session, mode: msg.mode }); } catch {}
        event("mode", { session: msg.session, mode: msg.mode });
        return reply(await popupState());
      }
      if (msg.t === "share") {
        const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
        const bad = tab && forbiddenUrl(tab.url);
        if (!tab || bad) return reply({ error: bad ? `Stitch does not work in ${bad}` : "no tab" });
        if (msg.session && state.sessions[msg.session]) {
          state.tabs[tab.id] = { session: msg.session, held: false, shared: true };
          await intoGroup(msg.session, tab.id);
        } else {
          // No session yet: offered, for the thread the user names to take.
          state.offered[tab.id] = true;
        }
        await save();
        event("share", { tab: tab.id, url: tab.url, title: tab.title, session: msg.session || null });
        return reply(await popupState());
      }
      if (msg.t === "pick") {
        // The picker: the user chooses an element in the active tab.
        const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
        const bad = tab && forbiddenUrl(tab.url);
        if (!tab || bad) return reply({ error: bad ? `Stitch does not work in ${bad}` : "no tab" });
        await toPage(tab.id, { t: "pick" }).catch(() => {});
        return reply({ picking: true });
      }
      return reply(null);
    }
    // From a page's content script: only about the tab it runs in.
    const tabId = sender.tab.id;
    if (msg.t === "picked") {
      // The user picked an element (any tab: they chose to point at it).
      if (!(await settings()).enabled || forbiddenUrl(sender.tab.url) || sender.frameId !== 0) return reply(null);
      if (!state.tabs[tabId]) state.offered[tabId] = true;
      await save();
      event("point", { tab: tabId, url: sender.tab.url, title: sender.tab.title, how: "picker",
        element: msg.element, ref: msg.ref, doc: msg.doc, selection: msg.selection || "",
        session: state.tabs[tabId]?.session || null });
      return reply(null);
    }
    const own = state.tabs[tabId];
    if (!own) return reply(null);
    // Frames (iframes) report the user's input; the rest is the top page's.
    if (msg.t === "user_input") {
      event("user_input", { tab: tabId, session: own.session, input: msg.kind, target: msg.target, key: msg.key, frame: sender.frameId });
      return reply(null);
    }
    if (sender.frameId !== 0) return reply(msg.t === "hello" ? { session: own.session, frame: true } : null);
    if (msg.t === "answer") {
      event("answer", { tab: tabId, session: own.session, id: msg.id, choice: msg.choice });
    } else if (msg.t === "user_input") {
      event("user_input", { tab: tabId, session: own.session, input: msg.kind, target: msg.target, key: msg.key });
    } else if (msg.t === "stop") {
      await stopSession(own.session, "Stop in the page");
    } else if (msg.t === "takeover") {
      own.held = true;
      await save();
      await detach(tabId);
      event("takeover", { tab: tabId, session: own.session });
    } else if (msg.t === "handback") {
      own.held = false;
      await save();
      const s = state.sessions[own.session];
      if (s?.group != null) await chrome.tabGroups.update(s.group, { title: s.title }).catch(() => {});
      const tab = await chrome.tabs.get(tabId).catch(() => null);
      event("handback", { tab: tabId, session: own.session, url: tab?.url, title: tab?.title });
    } else if (msg.t === "hello") {
      // A page (re)loaded in a session tab: it gets the overlay's state.
      const s = state.sessions[own.session];
      return reply({ session: own.session, title: s?.title, held: own.held, stopped: !!s?.stopped });
    }
    reply(null);
  })();
  return true;
});

async function popupState() {
  const tabs = await ops.tabs({});
  const sessions = Object.entries(state.sessions).map(([id, s]) => ({
    id, title: s.title, mode: s.mode, stopped: !!s.stopped, tabs: tabs.filter(t => t.session === id).length,
  }));
  return { profile: await settings(), connected: !!port, sessions };
}

// A tab dragged into a session's group joins it (the user gave it); one
// dragged out leaves it.
chrome.tabs.onUpdated.addListener(async (tabId, change, tab) => {
  await loaded;
  if ("groupId" in change && !grouping.has(tabId)) {
    const entry = Object.entries(state.sessions).find(([, s]) => s.group != null && s.group === change.groupId);
    const own = state.tabs[tabId];
    if (!own && entry) {
      const bad = forbiddenUrl(tab.url);
      if (!bad) {
        state.tabs[tabId] = { session: entry[0], held: false, shared: true };
        await save();
        event("share", { tab: tabId, url: tab.url, title: tab.title, session: entry[0] });
      }
    } else if (own && (!entry || entry[0] !== own.session)) {
      await detach(tabId);
      delete state.tabs[tabId];
      await save();
      event("tab_left", { tab: tabId, session: own.session });
    }
  }
  if (change.url) refused.delete(tabId);
  if (change.url && state.tabs[tabId]) event("url", { tab: tabId, url: change.url });
});

chrome.tabs.onRemoved.addListener(async tabId => {
  await loaded;
  if (state.offered[tabId]) {
    delete state.offered[tabId];
    await save();
  }
  if (!state.tabs[tabId]) return;
  const session = state.tabs[tabId].session;
  attached.delete(tabId);
  refused.delete(tabId);
  delete state.tabs[tabId];
  await save();
  event("tab_closed", { tab: tabId, session });
});

chrome.tabs.onActivated.addListener(i => event("activated", { tab: i.tabId, window: i.windowId, stitch: !!state.tabs[i.tabId] }));
chrome.windows.onFocusChanged.addListener(w => event("focus", { window: w }));
chrome.idle.setDetectionInterval(60);
chrome.idle.onStateChanged.addListener(s => event("idle", { state: s }));
