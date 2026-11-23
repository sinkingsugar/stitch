// The popup: the per-profile switch (§12.1: a profile is usable only once
// enabled here, by a click, never by a command), its name, and the
// sessions with Stop and the mode switch.

const $ = id => document.getElementById(id);

function render(st) {
  if (!st) return;
  $("enabled").checked = st.profile.enabled;
  if (document.activeElement !== $("name")) $("name").value = st.profile.name;
  $("dot").classList.toggle("on", st.connected);
  $("conn").textContent = st.connected ? "connected" : "no Stitch node";
  const box = $("sessions");
  box.textContent = "";
  if (!st.sessions.length) {
    box.innerHTML = '<span class="muted">No sessions.</span>';
  }
  for (const s of st.sessions) {
    const row = document.createElement("div");
    row.className = "session";
    const label = document.createElement("span");
    label.textContent = `${s.title} · ${s.tabs} tab${s.tabs === 1 ? "" : "s"}${s.stopped ? " · stopped" : ""}`;
    row.append(label);
    if (!s.stopped) {
      const mode = document.createElement("button");
      mode.textContent = s.mode === "alone" ? "Bring here" : "Own window";
      mode.onclick = async () => render(await chrome.runtime.sendMessage({ t: "mode", session: s.id, mode: s.mode === "alone" ? "coop" : "alone" }));
      const stop = document.createElement("button");
      stop.textContent = "Stop";
      stop.onclick = async () => render(await chrome.runtime.sendMessage({ t: "stopSession", session: s.id }));
      row.append(mode, stop);
    }
    box.append(row);
  }
  const live = st.sessions.filter(s => !s.stopped);
  $("share").hidden = !st.profile.enabled;
  // To the newest running session, or offered to whichever thread takes it.
  $("shareBtn").onclick = async () => render(await chrome.runtime.sendMessage({ t: "share", session: live.at(-1)?.id }));
  $("pickBtn").onclick = async () => {
    await chrome.runtime.sendMessage({ t: "pick" });
    window.close();
  };
}

$("enabled").onchange = async () =>
  render(await chrome.runtime.sendMessage({ t: "enable", on: $("enabled").checked, name: $("name").value.trim() }));
$("name").onchange = async () =>
  render(await chrome.runtime.sendMessage({ t: "enable", on: $("enabled").checked, name: $("name").value.trim() }));

chrome.runtime.sendMessage({ t: "popup" }).then(render);
setInterval(async () => render(await chrome.runtime.sendMessage({ t: "popup" })), 2000);
