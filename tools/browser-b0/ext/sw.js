// B0 spike: a relay between the node stub (over native messaging) and
// chrome.debugger / chrome.tabs / chrome.tabGroups. Plain JS, no bundler.

const HOST = "com.fragcolor.stitch.browser";
let port = null;
const attached = new Set();
const t0 = Date.now();

function log(...a) { console.log("[stitch-b0]", ...a); }

function send(msg) {
  if (port) port.postMessage(msg);
}

async function profileId() {
  const { id } = await chrome.storage.local.get("id");
  if (id) return id;
  const fresh = crypto.randomUUID();
  await chrome.storage.local.set({ id: fresh });
  return fresh;
}

async function connect() {
  if (port) return;
  try {
    port = chrome.runtime.connectNative(HOST);
  } catch (e) {
    log("connectNative threw", e);
    port = null;
    setTimeout(connect, 2000);
    return;
  }
  port.onMessage.addListener(onMessage);
  port.onDisconnect.addListener(() => {
    log("native port closed", chrome.runtime.lastError?.message);
    port = null;
    setTimeout(connect, 2000);
  });
  send({
    t: "hello",
    profile: await profileId(),
    ua: navigator.userAgent,
    sinceStart: Date.now() - t0,
  });
}

chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);
connect();

async function toTab(tab, msg) {
  try {
    return await chrome.tabs.sendMessage(tab, msg);
  } catch (e) {
    return { error: String(e?.message || e) };
  }
}

async function cdp(tab, method, params) {
  const input = method.startsWith("Input.");
  if (input) await toTab(tab, { t: "dispatch", open: true });
  try {
    return await chrome.debugger.sendCommand({ tabId: tab }, method, params || {});
  } finally {
    if (input) {
      // Events reach the page before the command's reply; a short grace
      // covers the content script's listeners running late.
      setTimeout(() => toTab(tab, { t: "dispatch", open: false }), 30);
    }
  }
}

const ops = {
  async newTab({ url, group, active, window }) {
    const tab = await chrome.tabs.create({ url, active: !!active, windowId: window });
    let groupId = null;
    if (group) {
      groupId = await chrome.tabs.group({ tabIds: [tab.id], groupId: group.id });
      await chrome.tabGroups.update(groupId, { title: group.title, color: group.color || "purple" });
    }
    return { tab: tab.id, group: groupId, window: tab.windowId };
  },
  async newWindow({ url, focused }) {
    const w = await chrome.windows.create({ url, focused: !!focused, state: "normal" });
    return { window: w.id, tab: w.tabs[0].id };
  },
  async attach({ tab }) {
    await chrome.debugger.attach({ tabId: tab }, "1.3");
    attached.add(tab);
    return {};
  },
  async detach({ tab }) {
    attached.delete(tab);
    await chrome.debugger.detach({ tabId: tab });
    return {};
  },
  cdp({ tab, method, params }) { return cdp(tab, method, params); },
  dispatch({ tab, open }) { return toTab(tab, { t: "dispatch", open }); },
  overlay({ tab, t, id, op, ...rest }) { return toTab(tab, { ...rest, t: "overlay" }); },
  async activate({ tab }) {
    const t = await chrome.tabs.update(tab, { active: true });
    return { tab: t.id };
  },
  async tabs() {
    const all = await chrome.tabs.query({});
    return all.map(t => ({ id: t.id, url: t.url, active: t.active, group: t.groupId, window: t.windowId }));
  },
  async capture({ window }) {
    // The visible tab of a window, as the extension sees it (no debugger).
    return { data: await chrome.tabs.captureVisibleTab(window, { format: "png" }) };
  },
  async moveGroup({ group, window }) {
    await chrome.tabGroups.move(group, { windowId: window, index: -1 });
    return {};
  },
};

async function onMessage(msg) {
  if (msg.t !== "req") return;
  try {
    const result = await ops[msg.op](msg);
    send({ t: "res", id: msg.id, ok: true, result });
  } catch (e) {
    send({ t: "res", id: msg.id, ok: false, error: String(e?.message || e) });
  }
}

chrome.debugger.onEvent.addListener((src, method, params) => {
  if (method.startsWith("Page.") || method === "Runtime.consoleAPICalled" || method === "Page.screencastFrame") {
    send({ t: "event", tab: src.tabId, method, params });
  }
});
chrome.debugger.onDetach.addListener((src, reason) => {
  attached.delete(src.tabId);
  send({ t: "detached", tab: src.tabId, reason });
});

chrome.runtime.onMessage.addListener((msg, sender) => {
  const tab = sender.tab?.id;
  if (msg.t === "user_input") send({ t: "user_input", tab, kind: msg.kind, at: msg.at, target: msg.target });
  if (msg.t === "stop") {
    if (attached.has(tab)) chrome.debugger.detach({ tabId: tab }).catch(() => {});
    send({ t: "stop", tab });
  }
});

chrome.idle.onStateChanged.addListener(state => send({ t: "idle", state }));
chrome.windows.onFocusChanged.addListener(w => send({ t: "focus", window: w }));
chrome.tabs.onActivated.addListener(i => send({ t: "activated", tab: i.tabId, window: i.windowId }));
