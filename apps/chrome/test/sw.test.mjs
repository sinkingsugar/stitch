// The service worker against a mock of Chrome's APIs, in Node:
// `node apps/chrome/test/sw.test.mjs` (crates/stitch-node/tests/extension.rs
// runs it). Its timings are shortened so a case takes well under a second.
// Exit status 0 when every case passes.

import { readFileSync } from "node:fs";
import vm from "node:vm";

const SRC = readFileSync(new URL("../sw.js", import.meta.url), "utf8");
const IDLE = 150;

function shorten(src, shotMs) {
  const sub = (re, to) => {
    if (!re.test(src)) throw new Error(`sw.js no longer has ${re}`);
    src = src.replace(re, to);
  };
  sub(/const DETACH_IDLE_MS = \d+;/, `const DETACH_IDLE_MS = ${IDLE};`);
  sub(/const IDLE_CHECK_MS = \d+;/, "const IDLE_CHECK_MS = 20;");
  sub(/const SHOT_TIMEOUT_MS = \d+;/, `const SHOT_TIMEOUT_MS = ${shotMs};`);
  return src;
}

const event = () => {
  const ls = [];
  return { addListener: f => ls.push(f), removeListener() {}, ls };
};

// A Chrome with tab 7 in session s1, and a debugger whose commands take
// `slowMs` (never answering when null); detaching fails what is in flight,
// as Chrome does.
function load({ slowMs, shotMs = 5000, patch }) {
  const sent = [];
  const log = [];
  let onMsg;
  const inFlight = new Set();
  let attachedNow = false;
  const timers = new Set();
  const store = {
    session: { state: { sessions: { s1: { title: "t", mode: "coop", window: null, group: null } }, tabs: { 7: { session: "s1", held: false, shared: false } }, offered: {}, downloads: {} } },
    local: { id: "p", name: "test", enabled: true },
  };
  const area = name => ({
    get: async keys => {
      const all = store[name];
      if (typeof keys === "string") return keys in all ? { [keys]: all[keys] } : {};
      return { ...all };
    },
    set: async v => Object.assign(store[name], v),
  });
  const chrome = {
    runtime: {
      id: "self",
      lastError: null,
      getManifest: () => ({ version: "0" }),
      connectNative: () => ({
        onMessage: { addListener: f => (onMsg = f) },
        onDisconnect: event(),
        postMessage: m => sent.push(m),
      }),
      onStartup: event(),
      onInstalled: event(),
      onMessage: event(),
      onConnect: event(),
    },
    storage: { session: area("session"), local: area("local"), onChanged: event() },
    contextMenus: { removeAll() {}, create() {}, onClicked: event() },
    tabs: {
      get: async id => ({ id, url: "https://example.com/", windowId: 1, active: false }),
      sendMessage: async () => ({ ok: true, result: {} }),
      query: async () => [],
      onUpdated: event(), onRemoved: event(), onActivated: event(), onCreated: event(), onReplaced: event(),
    },
    windows: { get: async () => ({ id: 1, state: "normal" }), onFocusChanged: event(), onRemoved: event() },
    tabGroups: { onUpdated: event(), onRemoved: event() },
    scripting: { executeScript: async () => [] },
    downloads: { onCreated: event(), onChanged: event(), onDeterminingFilename: event() },
    idle: { onStateChanged: event(), setDetectionInterval() {}, queryState: async () => "active" },
    action: { setBadgeText: async () => {}, setBadgeBackgroundColor: async () => {} },
    debugger: {
      onDetach: event(),
      onEvent: event(),
      attach: async () => {
        log.push("attach");
        attachedNow = true;
      },
      detach: async () => {
        log.push("detach");
        attachedNow = false;
        for (const f of inFlight) f(new Error("Detached while handling command."));
        inFlight.clear();
      },
      sendCommand: (_t, method) =>
        new Promise((ok, fail) => {
          if (!attachedNow) return fail(new Error("Debugger is not attached to the tab with id: 7."));
          if (!/captureScreenshot/.test(method)) return ok({});
          log.push("shot");
          inFlight.add(fail);
          if (slowMs != null) {
            const t = setTimeout(() => {
              inFlight.delete(fail);
              ok({ data: "png" });
            }, slowMs);
            timers.add(t);
          }
        }),
    },
  };
  patch?.(chrome, store);
  // Anything else the worker touches at load: a no-op event or function.
  const lenient = obj =>
    new Proxy(obj, {
      get(o, k) {
        if (k in o) return typeof o[k] === "object" && o[k] && !Array.isArray(o[k]) ? lenient(o[k]) : o[k];
        if (typeof k === "symbol" || k === "then") return undefined;
        return lenient(Object.assign(() => undefined, event()));
      },
    });
  const intervals = [];
  const ctx = {
    chrome: lenient(chrome),
    console,
    navigator: { userAgent: "test" },
    crypto: globalThis.crypto,
    setTimeout,
    clearTimeout,
    setInterval: (f, ms) => {
      const i = setInterval(f, ms);
      intervals.push(i);
      return i;
    },
    clearInterval,
    URL, fetch, Blob, TextEncoder, TextDecoder, atob, btoa,
  };
  vm.createContext(ctx);
  vm.runInContext(shorten(SRC, shotMs), ctx, { filename: "sw.js" });
  let next = 1;
  const call = (op, args) =>
    new Promise(done => {
      const id = next++;
      const wait = setInterval(() => {
        const r = sent.find(m => m.t === "res" && m.id === id);
        if (r) {
          clearInterval(wait);
          done(r);
        }
      }, 5);
      onMsg({ t: "req", id, op, ...args });
    });
  const close = () => {
    for (const i of intervals) clearInterval(i);
    for (const t of timers) clearTimeout(t);
  };
  return { call, log, close, store, sent, ctx };
}

// Windows and tabs as Chrome keeps them, for the tab and group cases:
// window 1 the user's (normal), window 9 a popup or an installed app's
// window, the last activated one: chrome.tabs.group without a window
// makes its group there, as Chrome does for a service worker
// (ChromeExtensionFunctionDetails::GetCurrentWindowController).
function windows(chrome, store, { capture = false } = {}) {
  const wins = { 1: { id: 1, type: "normal", state: "normal" }, 9: { id: 9, type: "app", state: "normal" } };
  const tabs = {};
  let nextWin = 20, nextTab = 100, nextGroup = 500;
  const groups = {};
  const open = (windowId, url) => {
    // An installed app capturing the URL into its own window.
    const t = { id: nextTab++, url, windowId: capture ? 9 : windowId, groupId: -1, active: false, status: "complete" };
    tabs[t.id] = t;
    return t;
  };
  Object.assign(chrome.windows, {
    get: async id => {
      if (!wins[id]) throw new Error(`No window with id: ${id}.`);
      return { ...wins[id] };
    },
    getLastFocused: async () => ({ ...wins[1], focused: true }),
    getAll: async () => Object.values(wins).filter(w => w.type === "normal"),
    create: async ({ url } = {}) => {
      const w = { id: nextWin++, type: "normal", state: "normal" };
      wins[w.id] = w;
      const t = open(w.id, url || "about:blank");
      return { ...w, tabs: [{ ...t }] };
    },
  });
  Object.assign(chrome.tabs, {
    get: async id => {
      if (!tabs[id]) throw new Error(`No tab with id: ${id}.`);
      return { ...tabs[id] };
    },
    create: async ({ windowId, url }) => ({ ...open(windowId, url) }),
    move: async (id, { windowId }) => {
      tabs[id].windowId = windowId;
      return { ...tabs[id] };
    },
    group: async ({ tabIds, groupId, createProperties }) => {
      const win = groupId != null ? groups[groupId] : createProperties?.windowId ?? 9;
      if (world.refuse || wins[win]?.type !== "normal") throw new Error("Grouping is not supported by tabs in this window.");
      const g = groupId ?? nextGroup++;
      groups[g] = win;
      for (const id of tabIds) Object.assign(tabs[id], { windowId: win, groupId: g });
      return g;
    },
  });
  Object.assign(chrome.tabGroups, {
    get: async id => {
      if (groups[id] == null) throw new Error(`No group with id: ${id}.`);
      return { id, windowId: groups[id] };
    },
    update: async () => ({}),
  });
  store.session.state.sessions.s1.mode = "alone";
  const world = { wins, tabs, groups, refuse: false };
  return world;
}

const cases = {
  // Issue w8dh: a screenshot slower than the idle time was detached under
  // by the idle timer, twice (the retry too): "Detached while handling command."
  async "a slow screenshot is not detached under by the idle timer"() {
    const w = load({ slowMs: IDLE * 3 });
    try {
      const r = await w.call("cdp", { tab: 7, method: "Page.captureScreenshot", params: {} });
      if (!r.ok) throw new Error(`failed: ${r.error} (debugger: ${w.log.join(", ")})`);
      if (r.result.data !== "png") throw new Error(`no image: ${JSON.stringify(r)}`);
      if (w.log.includes("detach")) throw new Error(`detached while busy: ${w.log.join(", ")}`);
      // Idle again afterwards: let go.
      await new Promise(ok => setTimeout(ok, IDLE * 3));
      if (!w.log.includes("detach")) throw new Error(`never let go once idle: ${w.log.join(", ")}`);
    } finally {
      w.close();
    }
  },

  async "a screenshot Chrome never makes fails with the reason, and the tab is let go"() {
    const w = load({ slowMs: null, shotMs: 200 });
    try {
      const r = await w.call("cdp", { tab: 7, method: "Page.captureScreenshot", params: {} });
      if (r.ok) throw new Error("succeeded");
      if (!/no screenshot of this tab in .*hidden or covered/.test(r.error)) throw new Error(`the wrong error: ${r.error}`);
      if (w.log.filter(x => x === "shot").length !== 1) throw new Error(`retried: ${w.log.join(", ")}`);
      if (!w.log.includes("detach")) throw new Error(`still attached: ${w.log.join(", ")}`);
    } finally {
      w.close();
    }
  },

  // Issue y53x: the session's first tab, in a window of its own, was
  // grouped with no window given, so in the last activated window (an
  // app's): "Grouping is not supported by tabs in this window", with the
  // tab open, registered in memory and never saved.
  async "a new tab is grouped in its own window, whatever window was active last"() {
    let world;
    const w = load({ slowMs: 0, patch: (c, st) => (world = windows(c, st)) });
    try {
      const r = await w.call("newTab", { session: "s1", url: "https://example.com/" });
      if (!r.ok) throw new Error(`failed: ${r.error}`);
      const t = world.tabs[r.result.tab];
      if (t.groupId === -1 || t.groupId !== r.result.group) throw new Error(`not grouped: ${JSON.stringify(t)}`);
      const s = w.store.session.state;
      if (t.windowId !== s.sessions.s1.window) throw new Error(`moved out of the session's window: ${JSON.stringify(t)}, session ${JSON.stringify(s.sessions.s1)}`);
      if (!s.tabs[t.id]) throw new Error("not saved");
      // A second one joins the group in the same window.
      const r2 = await w.call("newTab", { session: "s1", url: "https://example.com/b" });
      if (!r2.ok || world.tabs[r2.result.tab].groupId !== t.groupId) throw new Error(`second: ${JSON.stringify(r2)}`);
    } finally {
      w.close();
    }
  },

  async "a new tab an app window captured is moved back and grouped, and is saved either way"() {
    let world;
    const w = load({ slowMs: 0, patch: (c, st) => (world = windows(c, st, { capture: true })) });
    try {
      const r = await w.call("newTab", { session: "s1", url: "https://example.com/" });
      if (!r.ok) throw new Error(`failed: ${r.error}`);
      const t = world.tabs[r.result.tab];
      if (world.wins[t.windowId].type !== "normal" || t.groupId === -1) throw new Error(`left in the app window: ${JSON.stringify(t)}`);
      if (r.result.note) throw new Error(`a note: ${r.result.note}`);
      if (!w.store.session.state.tabs[t.id]) throw new Error("not saved");
      // Grouping refused even so: the call still gives the tab, saved, with a note.
      world.refuse = true;
      const r2 = await w.call("newTab", { session: "s1", url: "https://example.com/b" });
      if (!r2.ok) throw new Error(`failed: ${r2.error}`);
      if (!/not in the session's group/.test(r2.result.note || "")) throw new Error(`no note: ${JSON.stringify(r2.result)}`);
      if (!w.store.session.state.tabs[r2.result.tab]) throw new Error("the ungrouped tab is not saved");
    } finally {
      w.close();
    }
  },

  // Issue 4nwi: some window managers ignore focused:false and the new alone
  // window may have taken the user's keystrokes; another resized it between
  // calls. (A Node mock of Chrome, not Chrome.)
  async "the alone window is created at a fixed size and the user's window gets its focus back"() {
    const run = async prevFocused => {
      let world;
      const created = [], updates = [];
      const w = load({
        slowMs: 0,
        patch: (c, st) => {
          world = windows(c, st);
          c.windows.getLastFocused = async () => ({ ...world.wins[1], focused: prevFocused });
          const create = c.windows.create;
          c.windows.create = async opts => {
            created.push(opts);
            const r = await create(opts);
            world.wins[r.id].focused = true; // the WM took focus
            return { ...r, focused: true };
          };
          const get = c.windows.get;
          c.windows.get = async id => ({ ...(await get(id)), focused: world.wins[id].focused ?? false });
          c.windows.update = async (id, props) => {
            updates.push([id, props]);
            return {};
          };
        },
      });
      try {
        const r = await w.call("newTab", { session: "s1", url: "https://example.com/" });
        if (!r.ok) throw new Error(`failed: ${r.error}`);
        await new Promise(res => setTimeout(res, 50)); // nothing later either
        return { created, updates };
      } finally {
        w.close();
      }
    };
    const on = await run(true);
    const o = on.created[0];
    if (!o || o.focused !== false || o.width !== 1280 || o.height !== 900) throw new Error(`create options: ${JSON.stringify(on.created)}`);
    if (on.updates.length !== 1 || on.updates[0][0] !== 1 || on.updates[0][1].focused !== true) throw new Error(`the user's window was not refocused once: ${JSON.stringify(on.updates)}`);
    // Chrome was not focused (the user is in another app): never pull it to the front.
    const off = await run(false);
    if (off.updates.length) throw new Error(`refocused with Chrome unfocused: ${JSON.stringify(off.updates)}`);
    if (off.created[0]?.width !== 1280) throw new Error("no size");
  },

  // Issue y53x: right after a click followed a link, the read found no
  // script in the tab, and injecting one failed between documents with
  // Chrome's "Cannot access contents of the page".
  async "a page read between documents waits for the new one's script"() {
    let ready = false;
    const w = load({
      slowMs: 0,
      patch: c => {
        setTimeout(() => (ready = true), 400);
        c.tabs.get = async id => ({ id, url: "https://example.com/cart/", windowId: 1, status: ready ? "complete" : "loading" });
        c.tabs.sendMessage = async () => {
          if (!ready) throw new Error("Could not establish connection. Receiving end does not exist.");
          return { ok: true, result: { doc: "d2" } };
        };
        c.scripting.executeScript = async () => {
          throw new Error("Cannot access contents of the page. Extension manifest must request permission to access the respective host.");
        };
      },
    });
    try {
      const r = await w.call("page", { tab: 7, msg: { t: "info" } });
      if (!r.ok) throw new Error(`failed: ${r.error}`);
      if (r.result?.result?.doc !== "d2") throw new Error(`the wrong answer: ${JSON.stringify(r.result)}`);
    } finally {
      w.close();
    }
  },

  async "the context says whose Stitch tab the user's active tab is"() {
    const w = load({
      slowMs: 0,
      patch: c => {
        c.windows.getLastFocused = async () => ({ id: 1, focused: true });
        c.tabs.query = async () => [{ id: 7, url: "https://example.com/", title: "x" }];
      },
    });
    try {
      const r = await w.call("context", {});
      if (!r.ok || r.result.active?.session !== "s1") throw new Error(`no session: ${JSON.stringify(r)}`);
    } finally {
      w.close();
    }
  },

  // Issue e7qb: a click's download was missed (a trimmed referrer, a blob:
  // URL), so browser_downloads said there were none.
  async "a download with a trimmed referrer is the session tab's"() {
    let created;
    const w = load({
      slowMs: 0,
      patch: c => {
        created = c.downloads.onCreated;
        c.tabs.get = async id => ({ id, url: "https://example.com/docs/page?x=1", windowId: 1, active: false });
      },
    });
    try {
      await created.ls[0]({ id: 41, url: "https://cdn.example.net/f.pdf", referrer: "https://example.com/", filename: "/d/f.pdf" });
      if (!w.store.session.state.downloads?.[41]) throw new Error(`not matched: ${JSON.stringify(w.store.session.state.downloads)}`);
    } finally {
      w.close();
    }
  },

  async "a blob download right after an action is the acted tab's"() {
    let created;
    const w = load({
      slowMs: 0,
      patch: c => {
        created = c.downloads.onCreated;
      },
    });
    try {
      const r = await w.call("cdp", { tab: 7, method: "Input.dispatchMouseEvent", params: {} });
      if (!r.ok) throw new Error(`failed: ${r.error}`);
      await created.ls[0]({ id: 42, url: "blob:https://example.com/abc", referrer: "", filename: "/d/g.pdf" });
      if (!w.store.session.state.downloads?.[42]) throw new Error(`not matched: ${JSON.stringify(w.store.session.state.downloads)}`);
    } finally {
      w.close();
    }
  },
  // Issue dcez: two concurrent hello() calls (startup connect and a popup
  // enable) each found no id and made one, so a profile registered twice.
  async "concurrent hellos with no stored id share one profile id"() {
    let sets = 0;
    const w = load({
      slowMs: 0,
      patch: (c, store) => {
        delete store.local.id;
        const { get, set } = c.storage.local;
        const tick = () => new Promise(r => setTimeout(r, 2));
        c.storage.local = {
          get: async k => (await tick(), get(k)),
          set: async v => {
            if ("id" in v) sets++;
            await tick();
            return set(v);
          },
        };
      },
    });
    try {
      await Promise.all([w.ctx.hello(), w.ctx.chrome.storage.local.set({ enabled: true, name: "test" }).then(w.ctx.hello), w.ctx.hello()]);
      const ids = w.sent.filter(m => m.t === "hello").map(m => m.profile.id);
      if (ids.length < 3 || new Set(ids).size !== 1) throw new Error(`hellos disagree: ${JSON.stringify(ids)}`);
      if (sets !== 1 || w.store.local.id !== ids[0]) throw new Error(`ids stored: ${sets}, kept ${w.store.local.id} vs ${ids[0]}`);
    } finally {
      w.close();
    }
  },
};

let failed = 0;
for (const [name, run] of Object.entries(cases)) {
  try {
    await run();
    console.log(`ok   ${name}`);
  } catch (e) {
    failed++;
    console.log(`FAIL ${name}: ${e?.stack || e}`);
  }
}
process.exit(failed ? 1 : 0);
