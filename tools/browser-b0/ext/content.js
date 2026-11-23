// B0 spike: the in-page overlay (glow, agent cursor, label, Stop) and the
// user-input watcher. CDP input is isTrusted too, so events inside the
// agent's dispatch windows are the agent's; anything else is the user.

// A count, not a flag: the close of one command's window (sent 30 ms late)
// must not close the window of the next command already in flight.
let dispatching = 0;
let host = null;
let ui = null;

function ensureOverlay() {
  if (host) return;
  host = document.createElement("stitch-overlay");
  const root = host.attachShadow({ mode: "closed" });
  root.innerHTML = `
    <style>
      .glow { position: fixed; inset: 0; pointer-events: none; z-index: 2147483646;
              box-shadow: inset 0 0 0 3px rgba(150, 90, 255, .9), inset 0 0 24px rgba(150, 90, 255, .5); }
      .cursor { position: fixed; width: 14px; height: 14px; border-radius: 50%; background: rgba(150,90,255,.9);
                pointer-events: none; z-index: 2147483647; transition: left .25s, top .25s; left: -50px; top: -50px; }
      .label { position: fixed; pointer-events: none; z-index: 2147483647; font: 12px system-ui; color: #fff;
               background: rgba(80, 40, 160, .9); padding: 2px 6px; border-radius: 4px; left: -500px; top: -500px; }
      .pill { position: fixed; bottom: 12px; left: 50%; transform: translateX(-50%); z-index: 2147483647;
              font: 13px system-ui; background: #2b1a55; color: #fff; padding: 6px 10px; border-radius: 16px; }
      .pill button { margin-left: 8px; }
    </style>
    <div class="glow"></div><div class="cursor"></div><div class="label"></div>
    <div class="pill"><span class="state">Stitch is working</span><button class="stop">Stop</button></div>`;
  ui = {
    root,
    cursor: root.querySelector(".cursor"),
    label: root.querySelector(".label"),
    state: root.querySelector(".state"),
  };
  root.querySelector(".stop").addEventListener("click", e => {
    e.stopPropagation();
    chrome.runtime.sendMessage({ t: "stop" });
    ui.state.textContent = "Stopped";
  });
  (document.body || document.documentElement).appendChild(host);
}

chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
  if (msg.t === "dispatch") {
    dispatching = Math.max(0, dispatching + (msg.open ? 1 : -1));
    reply(true);
  } else if (msg.t === "overlay") {
    try { overlay(msg); reply(true); } catch (e) { reply({ error: String(e) }); }
  }
});

function overlay(msg) {
  {
    if (msg.hide) {
      if (host) host.style.display = "none";
    } else {
      ensureOverlay();
      host.style.display = "";
      if (msg.x != null) {
        ui.cursor.style.left = msg.x - 7 + "px";
        ui.cursor.style.top = msg.y - 7 + "px";
        ui.label.style.left = msg.x + 12 + "px";
        ui.label.style.top = msg.y + 8 + "px";
      }
      if (msg.label != null) ui.label.textContent = msg.label;
      if (msg.state != null) ui.state.textContent = msg.state;
    }
  }
}

function describe(t) {
  if (!(t instanceof Element)) return "";
  return t.tagName.toLowerCase() + (t.id ? "#" + t.id : "") + (t.name ? `[name=${t.name}]` : "");
}

for (const kind of ["pointerdown", "keydown", "wheel", "paste"]) {
  window.addEventListener(
    kind,
    e => {
      // The agent's CDP input carries `timestamp: 1` (1970), which reaches the
      // page as event.timeStamp 0: no real input has that. Dispatch windows
      // are kept only as a log of the old approach (MARK=0 in driver.mjs).
      if (!e.isTrusted || e.timeStamp === 0 || (globalThis.__stitchWindows && dispatching)) return;
      if (host && e.composedPath().includes(host)) return; // our own buttons
      chrome.runtime.sendMessage({ t: "user_input", kind, at: Date.now(), target: describe(e.target) });
    },
    { capture: true, passive: true }
  );
}
