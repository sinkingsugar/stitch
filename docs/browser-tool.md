# Browser control: plan

Status: **decided 2026-10-17** (§12; recorded in design.md §2.18). **B0, the spike, done 2026-10-18 (§15): the extension path holds.** B1 is under way on the `browser-tool` branch. This plan is for a browser tool that matches Claude in Chrome and OpenAI's browser agents (Operator, ChatGPT agent, Atlas).

**The user's direction (2026-10-17):** the top priority is driving the user's **own Chrome**, with its real profile and logins, and **cooperative sessions** in which the user and the agent work in the same browser at the same time. The headless browser and profiles Stitch launches are secondary backends that come later.

## 0. Summary

- **The primary backend is the user's running Chrome, reached through an extension of Stitch's own.** The extension uses `chrome.debugger` (CDP: the DevTools protocol) and native messaging to the local Stitch node. This is the model Claude in Chrome uses, and its installed copy on this Mac was studied for this plan (§3).
- **Chrome 144+'s consent attach is the zero-install fallback.** It needs a click in Chrome for each connection, reaches the whole browser, and cannot do tab groups or offer a "this tab" button. Because the engine is the same, it is only a transport on top.
- **One engine, in Rust in the node, on CDP.** It handles tabs, accessibility snapshots with element refs, screenshots, trusted input, waiting, dialogs, uploads and downloads. Both mechanisms are only transports under it: the extension relays CDP per tab, and the consent attach is a WebSocket. Later backends (headless, managed profiles, a CDP address, another node) plug in the same way.
- **Cooperative mode is the default in the user's Chrome.** The agent works in a tab group of its own, coloured and titled, in the user's window:
  - an in-page overlay shows a glow, the agent's cursor, what it is doing, and Stop / Take over buttons;
  - **the user always wins:** any input from the user in an agent tab pauses the agent there;
  - every action is checked against the page's epoch, so the agent re-reads whatever the user changed before it acts;
  - the user can point the agent at a tab, a selection or an element: "this tab", "this form".
- **Agent-alone mode** runs in the same Chrome, in a window of its own that the user can minimise. The agent never steals focus. A click on the overlay, the toolbar, or `/browser` in a chat switches between the two modes.
- **Safety is built for a real profile:**
  - the agent touches only its own tabs, and the extension enforces it in the browser;
  - per-site grants, plus a fixed set of consequential actions that always ask (yolo included);
  - "watching" (cooperative, the agent's tab in front, the user active) lets in-site actions run without asking, and the questions that remain are asked in the page;
  - with nobody watching, questions go to the user's devices, phone included;
  - devtools-style tools are off; memory is off; page content is fenced as untrusted.
- **Order (§11):**
  1. B0, a spike of both mechanisms with a minimal cooperative loop;
  2. B1, cooperative sessions in the user's Chrome;
  3. B2, the phone: watching, taking over and driving across nodes (the live channel, pulled forward);
  4. then the consent fallback, the Web Store, the secondary backends, and recording.

## 1. What Stitch has today

- **Web tools** (`crates/stitch-node/src/web/`, design §2.3):
  - `browse_web` reads a page or a file;
  - `web_eval` runs JavaScript in a page once;
  - `search_web` searches.
- **The headless browser:** one Chromium per node, driven over CDP with `chromiumoxide` (`chrome.rs`). It starts with the first call, is shared, and stops after 5 idle minutes. It runs the installed Chrome, Chromium, Edge or Brave, or Chrome for Testing's headless shell (`stitch browser install`). It has a throwaway profile and a 1280×900 viewport at DPR 1.
- **Each call gets a fresh browser context** (`page.rs`): no cookies, downloads denied, dialogs dismissed. There are no sessions, tabs, input actions, refs or logins.
- **Google Flights** (an Otter skill tool) reads HTML over `Http.Get` and uses no browser. Otter has no browser shards.
- **Tool hosting:**
  - built-in groups per thread through toolsets (§2.14);
  - MCP tools (§2.13) and Otter tools (§2.17);
  - `ToolOutcome` carries text and images; images go to blobs with `img:` IDs, so `zoom_image` works on them.
- **Approvals** (`tools.rs` `Asker`):
  - the answers are allow, always (a place grant) or decline;
  - detached threads ask through `question` records, answered from any client, and pushed sealed to the phone;
  - read-only threads are refused; yolo allows.
- **Clients and sockets:** the node serves local clients on a Unix socket in its data directory (§2.1). That is where a native messaging host will connect.
- **Tools on another node** (§2.14): shared MCP servers are published in the `capabilities` record and called over `stitch/node/1` `Ask`, under that node's rules.
- **The phone** starts no Chrome (its `browse_web` and `web_eval` render in the app's WebKit, design §2.3; the user's own browser is a desktop's). It answers approvals and shows tool images. There is no live channel yet (milestone 6).
- **On this Mac:**
  - Chrome 155 (and Brave) are installed;
  - **six Chrome profiles** (the Default one and five others);
  - Claude in Chrome 1.0.100/1.0.101, in the Default profile only, with Claude Code's native host (`com.anthropic.claude_code_browser_extension` → `~/.claude/chrome/chrome-native-host`, a shell script that runs `claude --chrome-native-host`);
  - remote debugging is currently off (no `DevToolsActivePort`).

## 2. What edge talk learned

edge talk drove a WebKit view in its app with injected JavaScript. It had no CDP and no tabs (`EdgeTalk/Shards/WebViewInteractionShard.swift`, `WebViewCommon.swift`, `Scripts-Src/Tools/tools.shs:218–570`).

- **Keep:**
  - one coordinate space for screenshots, clicks and errors (`6075b07b`, `448021f8`);
  - zoom before clicking small targets, a step descriptions must require (`1b45e13d`), with errors echoing the model's coordinates (`b21e9954`);
  - structure over pixels;
  - framework-safe value setting (`6075b07b`);
  - finding the right scroll container (`986e5b8a`);
  - a "practically loaded" check (`domCheckScript`);
  - the eval wrapper (survives a CSP, collects the console, maps error lines);
  - telling the agent when the user moved the page (`[User navigated to: …]`, `6821cd6a`). This is the seed of §6's change detection.
- **Do differently:**
  - synthetic DOM events are untrusted, and native ones moved the real cursor: use CDP `Input.*`;
  - selectors went stale: use refs bound to DOM node IDs;
  - there were no tabs, uploads, downloads or accessibility tree;
  - one cookie store was shared between the user and the agent without the user deciding it. Here sharing the user's profile is the point, so it is explicit, scoped to the agent's tabs, and visible;
  - safety was wording only: here it is real approvals (§8).

## 3. The bar: what the others do

**Sources:**
- Claude in Chrome: [overview](https://claude.com/resources/articles/claude-for-chrome), [get started](https://support.claude.com/en/articles/12012173-get-started-with-claude-in-chrome), [safety](https://support.claude.com/en/articles/12902428-use-claude-in-chrome-safely), [permissions](https://support.claude.com/en/articles/12902446-claude-in-chrome-permissions-guide), [Claude Code `--chrome`](https://code.claude.com/docs/en/chrome), [injection defenses](https://www.anthropic.com/research/prompt-injection-defenses).
- Anthropic's API: [browser use tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/browser-use-tool), [computer use tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool).
- OpenAI: [CUA](https://openai.com/index/computer-using-agent/), [ChatGPT agent system card](https://cdn.openai.com/pdf/839e66fc-602c-48bf-81d3-b21eacc3459d/chatgpt_agent_system_card.pdf), [ChatGPT agent help](https://help.openai.com/en/articles/11752874-chatgpt-agent), [Atlas](https://openai.com/index/introducing-chatgpt-atlas/), [Atlas agent help](https://help.openai.com/en/articles/12628199-using-ask-chatgpt-sidebar-and-chatgpt-agent-on-atlas), [Responses computer use](https://developers.openai.com/api/docs/guides/tools-computer-use), [Atlas shut down](https://techcrunch.com/2026/07/09/openai-is-shutting-down-atlas-but-its-ai-browser-ambitions-are-still-growing/).
- Comparables: [Playwright MCP](https://github.com/microsoft/playwright-mcp), [Chrome DevTools MCP tools](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/main/docs/tool-reference.md), [browser-use](https://github.com/browser-use/browser-use).
- Chrome: [remote debugging and the default profile (136)](https://developer.chrome.com/blog/remote-debugging-port), [consent attach (144+)](https://developer.chrome.com/blog/chrome-devtools-mcp-debug-your-browser-session), [`chrome.debugger`](https://developer.chrome.com/docs/extensions/reference/api/debugger), [service worker lifecycle](https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle), [the debugger infobar in Brave](https://community.brave.app/t/option-to-disable-the-extension-started-debugging-this-browser-infobar/658856).
- OpenAI's pages answered 403 to the headless browser, so some OpenAI points come from snippets and the system card. Atlas shut down on 9 Aug 2026; ChatGPT agent is reported folded into "ChatGPT Work" (not verified).

**Claude in Chrome, read from the installed extension** (1.0.100/101: `manifest.json`, `assets/mcpPermissions-*.js`, `service-worker.ts-*.js`, `agent-visual-indicator.js-*.js`, `accessibility-tree.js-*.js`):
- **Tools:**
  - tabs: `tabs_context` and `tabs_create` (a tab group per session);
  - navigation: `navigate`;
  - reading: `read_page` (accessibility tree with `ref_N`, an interactive filter, depth or one subtree, at most 50,000 characters), `find`, `get_page_text`;
  - input: `form_input`, and `computer` (`left_click`, `right_click`, `double_click`, `triple_click`, `left_click_drag`, `hover`, `type`, `key`, `scroll`, `scroll_to`, `wait`, `screenshot`, `zoom`);
  - developer: `javascript_tool`, `read_console_messages`, `read_network_requests`;
  - files and window: `file_upload`, `upload_image`, `resize_window`;
  - other: `gif_creator`, `update_plan`, `shortcuts_list` / `shortcuts_execute`, `browser_batch`.
- **Modes:** `ask`, `follow_a_plan` and `skip_all_permission_checks`, plus the automatic (classifier) mode. Admin `blockedUrlPatterns`.
- **How it is built** (what this plan copies in kind):
  - **Transport:** the MV3 service worker calls `chrome.runtime.connectNative` on known host names in turn (`com.anthropic.claude_browser_extension` for Desktop, `com.anthropic.claude_code_browser_extension` for Claude Code), pinging each. Tabs are driven with `chrome.debugger.attach({tabId}, "1.3")` and `sendCommand`. The attach is serialised per tab and times out ("DevTools may be open").
  - **Its own accessibility tree:** a content script on all frames at `document_start` (`accessibility-tree.js`, 7 KB) builds it, rather than CDP's `Accessibility` domain.
  - **Overlays** from a content script (`agent-visual-indicator.js`):
    - a glow border (`claude-agent-glow-border`);
    - a **phantom cursor** that animates to each click (`claude-phantom-cursor`);
    - a **"Stop Claude" button** at the bottom centre;
    - a static pill with a heartbeat on the group's tabs while idle;
    - all hidden while a screenshot is taken (`HIDE_FOR_TOOL_USE` / `SHOW_AFTER_TOOL_USE`), so they never show in what the model sees.
  - **Background tabs:** it works in tabs that are not in front with `Emulation.setFocusEmulationEnabled` ("focus emulation") and a "hidden tab draw" workaround for tabs Chrome stops painting.
  - **Tab group:** a per-session tab group (`chrome.tabGroups.update` with title and colour).
  - **Profile reach:** installed per profile; on this Mac only in Default.
  - **Infobar:** while attached, Chrome shows its "… started debugging this browser" infobar. Users ask browsers to let them hide it; Chrome does not.
- **OpenAI:**
  - screenshots in a remote browser that starts logged out;
  - takeover (no screenshots while the user drives);
  - watch mode (pauses on sensitive sites when the user looks away);
  - trained confirmations;
  - memory off;
  - Atlas: a logged-in or logged-out mode, no code, downloads or extensions.

## 4. Attaching to the user's Chrome

Since Chrome 136, `--remote-debugging-port` is ignored on the default profile (anti-infostealer: a non-default data directory gets its own encryption key). That leaves two ways into the user's real profile:

| | (a) Stitch's own extension (`chrome.debugger` + native messaging) | (b) consent attach (Chrome 144+, `chrome://inspect/#remote-debugging`) |
|---|---|---|
| Consent | once: install, plus Chrome's permission prompt (it reads and changes all sites, debugger) | remote debugging switched on once, then **Chrome's Allow dialog on every connection** |
| Survives a Chrome restart | yes: `runtime.onStartup` reconnects the native port; a native port keeps the MV3 service worker alive (Chrome 105+), and so do debugger sessions (118+) | the connection dies with Chrome; the next one asks again. Whether the toggle itself persists is to be checked in B0. The node keeps one connection and reconnects only when it must, so it is one click per Chrome or node restart |
| Survives a node restart | yes: the host process reconnects to the new node | asks again |
| Unattended (detached thread at night) | yes | only if a connection is already open; otherwise it waits for a click nobody gives |
| Several Chrome profiles | per profile: install in each wanted profile; each registers as its own browser (`chrome:work`), and profiles without it are out of reach, which is a feature | one connection reaches every profile open in that Chrome process (to be confirmed in B0); scoping is all Stitch's |
| Scope enforced by | the browser: the extension attaches only to the agent's tabs and refuses others | Stitch only: CDP sees every tab, cookie and profile |
| Working while the user uses Chrome | designed for it: tab group in the user's window, focus emulation, nothing activated | works (CDP drives background targets), but there is no tab-group API, no toolbar, no context menu, and "active tab" must be guessed from visibility |
| Showing what the agent does | tab group title, colour and badge; toolbar badge; in-page overlay (glow, cursor, Stop / Take over); notifications | in-page overlay only (injected into an isolated world); Chrome's "controlled by automated test software" banner on the whole browser while connected |
| Pointing ("this tab", "this form") | toolbar button, context menu on a page, selection or element, keyboard shortcut | none: the user says it in chat and the agent guesses (the visible tab, the focused element) |
| Banner | "Stitch started debugging this browser" infobar while attached to any tab (Chrome's, cannot be hidden); detach when idle hides it | automation banner while connected |
| CDP reach | per tab: Accessibility, DOM, DOMSnapshot, Emulation, Fetch, Input, Network, Overlay, Page, Runtime, Target (auto-attach to iframes) and more; no Browser domain, so tabs and windows go through `chrome.tabs` / `chrome.windows` | everything |
| Other Chromium browsers | Brave, Edge, Arc, Vivaldi: the same extension, with a host manifest per browser | where they ship the 144 flow (unknown) |
| What to build | MV3 extension (service worker relay, content script overlay and input watcher, popup and context menu; plain JS, no bundler), `stitch browser-host` (native messaging ↔ node socket), installer | a transport on top of the engine |
| Distribution | **sideloaded** ("Load unpacked" from the files `stitch browser install` writes, developer mode on; a pinned `key` gives a stable ID for the host manifest's `allowed_origins`), then an **unlisted Web Store** listing for auto-updates (review takes days; `debugger` and `<all_urls>` get a closer look). Off-store `.crx` installs are blocked on macOS and Windows without MDM | nothing to distribute |

**Recommendation: (a), the extension, as the primary path, with (b) as the fallback.**
- Cooperative work needs what only an extension has: tab groups, a toolbar and context menu to point with, scoping the browser itself enforces, and a connection that comes back by itself after restarts with no click.
- The per-connection dialog of (b) is right for a debugging session and wrong for a browser the agent lives in, particularly for detached threads.
- (b) shares the engine, so it is cheap to keep for Brave without the extension, for a machine where the extension is not installed, and for the B0 comparison.

**Also considered:**
- **Talking to Claude in Chrome:** it accepts only Anthropic's native hosts and signs in to Anthropic, so it is another product's private protocol.
- **Copying the user's profile into a Stitch-launched Chrome:** the infostealer pattern Chrome closed. No.
- **Playwright and `connectOverCDP`:** a second runtime, and still (b).

## 5. Architecture

### 5.1 Pieces

```
 Chrome (user's profile)                      Stitch node (Rust)
 ┌───────────────────────────────┐           ┌──────────────────────────────────┐
 │ extension (MV3)               │  native   │ browser/                         │
 │  service worker: CDP relay,   │ messaging │  registry: browsers by profile   │
 │   tabs, tabGroups, windows,   │──stdio──▶ │  engine: sessions, tabs, refs,   │
 │   idle, contextMenus          │ stitch    │   snapshot, input, wait, epochs  │
 │  content script: overlay,     │ browser-  │  coop: driver arbitration        │
 │   user-input watcher, picker  │ host ──▶  │  transports: Extension | Cdp(ws) │
 └───────────────────────────────┘ node sock └──────────────────────────────────┘
```

- **`stitch browser-host`** is a subcommand of the one binary (§2.1). Chrome starts it per profile when the extension calls `connectNative("com.fragcolor.stitch.browser")`. It finds the node of its data directory (starting one if none runs, as every client does), and relays length-prefixed JSON between stdio and a `BrowserLink` stream on the node's local socket. It holds no state: a node or host restart only reconnects.
  - The host manifest is installed for Chrome, Brave and Edge by `stitch browser install` (the headless Chromium download it did before is now `stitch browser install headless`), with `allowed_origins` limited to the extension's pinned ID.
- **The extension** is a thin relay plus UI, with no model, no account and no server.
  - Its service worker forwards CDP commands and events per attached tab (`chrome.debugger`), and runs tab, window and group operations (`chrome.tabs`, `chrome.tabGroups`, `chrome.windows`).
  - It reports what the user does (§6.3).
  - It shows the UI: group title and colour, toolbar badge, popup, context menu, notifications.
  - It **refuses to attach to any tab the node has not been given** (§6.1), so a confused or compromised node still cannot read the user's other tabs.
  - On hello it sends its profile identity: a random ID kept in `chrome.storage.local`, the profile's display name if the user sets one in the popup, the browser and its version.
- **The engine** (`crates/stitch-node/src/browser/`, desktop only) speaks CDP over a `Transport` trait: `send(target, method, params)`, an event stream, plus tab operations (create, close, activate, group, move to a window).
  - `chromiumoxide` assumes it owns a browser-level WebSocket, so the engine uses its generated protocol types (`chromiumoxide_cdp`) over a session layer of its own.
  - Transports: `Extension` (per-tab sessions relayed), `Cdp` (a WebSocket with flattened `Target.attachToTarget` sessions, for the consent attach and later backends).
  - The existing web tools keep `chromiumoxide` as they are.

### 5.2 Sessions, tabs and refs

- **A browser session belongs to a thread and a browser** (a Chrome profile, `chrome:<name>`). It is kept in a `Browsers` registry, as `Shells` are.
- It holds:
  - its **tab group**: in cooperative mode, in the user's current window, titled `Stitch · <thread title>` in a colour per thread;
  - **tabs it opened, or that the user gave it** (§6.4);
  - per tab: the ref table, the epoch (§6.2), a console ring and a network ring (when on), the held dialog, downloads, and recording frames.
- **Lifetime:**
  - the session ends when the thread ends or is cancelled, or after `idle_minutes`;
  - its tabs are **left open** for the user, ungrouped (default, §12);
  - the debugger detaches when idle, so Chrome's infobar goes away: 20 s after a tab's last CDP command ended, never while one is in flight (issue w8dh, 2026-10-31: a screenshot slower than that was detached under, twice with the retry, and failed with "Detached while handling command.").
  - A screenshot Chrome does not make in 25 s (it draws nothing for a tab whose window is hidden or covered, e.g. by a full-screen game) fails with that reason, and the tab is let go, dropping the command Chrome still waits on.
  - After a node restart the browser re-registers, and the thread finds its tabs again by the IDs it kept (tab IDs live as long as Chrome does). After a Chrome restart, its tabs are gone, and the thread is told.
- **Refs** are `ref_N` per tab, bound to a `backendDOMNodeId`:
  - read with `DOMSnapshot.captureSnapshot` / `Accessibility.getFullAXTree` per frame, with out-of-process iframes through auto-attach;
  - resolved at action time (`DOM.resolveNode`, `DOM.scrollIntoViewIfNeeded`, `DOM.getContentQuads`);
  - invalidated on navigation;
  - lines in Claude's form (`button "Search" [ref_4]`), password values masked.
  - Claude builds its tree in a content script instead. B0 compares the two approaches on real sites.

### 5.3 Observation, input and waiting

- **`browser_snapshot`:** the tree (interactive filter, depth, one subtree, `max_chars` 32,000 with the rest saved), or `text: true` for the main text.
- **`browser_screenshot`:** the viewport, the full page or one ref; `region` zooms.
  - It returns an `img:` image, the scale, and the mapping formula.
  - The overlay is hidden for the capture, as Claude does.
  - **Background tabs:** `Page.captureScreenshot` on a tab that is not in front can return stale or blank frames. The workaround to prove in B0 is focus emulation plus a forced frame: a `Page.startScreencast` frame, or a short repaint. `HeadlessExperimental.beginFrame` is not an option, since it exists only in headless Chrome.
- **Input** through CDP `Input.*` (trusted; it never moves the user's cursor):
  - mouse and keys (`dispatchMouseEvent`, `dispatchKeyEvent`), and `insertText` for long text;
  - form values set by ref, firing `input` and `change`;
  - uploads with `DOM.setFileInputFiles`, never a native picker (`Page.setInterceptFileChooserDialog`);
  - dialogs held and reported.
- **Actions never activate a tab or window, or focus Chrome,** in cooperative mode: focus emulation makes a background tab behave as focused.
- **Waiting:** each action waits for its effect (navigation committed, then settled: load, 500 ms without DOM changes, at most 5 s, no visible spinner). `browser_wait` waits for a selector, text, URL, network idle or a time, 30 s by default and at most 120 s.

### 5.4 Secondary backends (later phases)

The same engine with `Cdp` transports:
- `ephemeral`: today's headless Chromium, made into sessions; for logged-out, throwaway work;
- `managed`: a profile Stitch launches with its own data directory, for an always-on box with no user at it;
- `connect`: any CDP address, such as a cloud browser.
- **Profiles on another node** are shared like MCP servers (§2.14): `[browser] share = ["chrome:work"]` publishes them in `capabilities`, so a phone thread can drive the Mac's Chrome, under the Mac's rules. Since nobody is at that Chrome when the phone drives, sessions from another node start in agent-alone mode (§6.5) unless the user at the Mac picks them up.

```toml
[browser]
default = "chrome"              # the user's Chrome through the extension (any connected profile; asks which when several)
idle_minutes = 10
share = []                      # browsers other nodes may drive (B3)

[browser.chrome]                # the extension and consent attach
consent_attach = false          # true: also try Chrome 144+'s consent attach when no extension is connected
# No profile list here: profiles are enabled by a click in each profile's extension popup (§12.1).

[browser.profiles.scratch]      # secondary backends (B5)
kind = "ephemeral"
```

## 6. Cooperative sessions

### 6.1 What the user sees

- **A tab group of the agent's own,** in the window the user is in: titled `Stitch · <thread>`, in its own colour, with the badge `…` while acting, `‖` paused, `?` waiting on the user.
- **On each of its tabs**, an overlay (content script, shadow DOM, `pointer-events: none` except its buttons):
  - a thin glow border;
  - the **agent's cursor** animating to each target, with a short label of the action ("typing in Search", "clicking Submit");
  - a pill with **Stop**, **Take over** / **Hand back**, and the thread's name (click: open the thread in a Stitch client).
  - It is hidden in screenshots. Questions asked while the user watches show here too (§8.4).
- **The toolbar icon:**
  - the badge counts active sessions;
  - the popup lists sessions (thread, mode, state) with Stop, Pause, mode switch, "share this tab", and the profile's name.
- **Chrome's own infobar** while attached. It is not optional. The debugger is attached only to the tabs the agent works in, and detached when it is idle, so the bar comes and goes with real activity.

### 6.2 Noticing what the user changed

Every tab has an **epoch**, a counter the engine increments when something the agent did not cause happens:
- a navigation or reload it did not start;
- the user's trusted input in the tab (§6.3);
- a different focused element, selection or scroll position;
- the active tab of the group changing, or a tab added to, closed in or moved out of the group;
- large DOM changes after the agent's last action had settled (a `MutationObserver` count past a threshold; small live updates such as clocks and counters are ignored).

**Agent-caused changes are attributed:** everything inside an action's window, from the first input event until it settles, belongs to the agent and does not bump the epoch.

Each observation result carries `epoch: N`, and the engine remembers the last epoch the model saw per tab. **An action on a tab whose epoch has moved since does not run.** It fails with what changed ("the user navigated to …", "the user typed in `email`", "the active tab is now …"), plus a fresh interactive snapshot when that costs under 4,000 characters, so the model re-plans from the truth. Refs from the old snapshot stay valid only if their nodes still exist and the page did not navigate.

### 6.3 Who drives

**The user always wins.** Everything follows from that one rule; there is no lock the user has to ask for.

- **User input:** the content script listens in capture phase for `pointerdown`, `keydown`, `wheel`, `input` and `paste`. CDP input is also `isTrusted`, so the watcher ignores events during the agent's own dispatch windows (the node tells the extension when one opens and closes, and a CDP event carries the target coordinates for a cross-check). Anything else is the user. It is sent to the node as `user_input {tab, kind, at}`.
- **On user input in an agent tab:**
  1. the agent's batch there stops after the current atomic event (never halfway through a key chord), and the remaining actions are dropped;
  2. the tab is **paused**; the badge and overlay show it;
  3. the action's result says the user took the tab: "paused: the user is typing in `email`".
  4. The engine **resumes** when the user has been idle in that tab for `resume_after` (default 4 s) **and** the agent re-reads (the epoch moved, so its next action needs a fresh observation anyway). A call that would act while the tab is paused waits up to its timeout, then fails with "the user is using this tab".
- **Take over:** the overlay's **Take over** button (or a long pause: typing into a password field, which always triggers it, §8) holds the tab until **Hand back**.
  - Meanwhile no screenshots or snapshots of that tab go to the model (OpenAI's rule).
  - `browser_handoff` is the agent asking for exactly this ("log in, then hand back"). It waits until Hand back, or the call's limit (at most 30 minutes for a handoff; asked again after that).
  - The handoff is also a question in every Stitch client (the phone included), worded as "answer when you are done", not as a permission: answered there, the node tells the extension to `release` the tab, as Hand back does (before, the extension kept holding it and every read was refused).
  - A handoff that runs out its limit also tells the extension to `release` the tab and clears the hold, and the error says the tab is back with you (before, it stayed held and every later call was refused as "taken over"). The user's key in the page is named in the change note (`(key "w")`).
  - `browser_fill` on an ARIA `checkbox`/`switch`/`radio` (not a native input) toggles it with a click to reach the requested state; the value setter is used only on `INPUT`/`TEXTAREA`; any other element is refused as "not a fillable field" (it used to fail with "Illegal invocation").
  - Download matching in the extension is tolerant: a download is the session's if its referrer origin is a session tab's, or a recent action of the session began within 30 s; tests in `apps/chrome/test/sw.test.mjs` and `content.test.mjs` (run by `crates/stitch-node/tests/extension.rs`).
  - A terminal prompt that appears while the user types a message does not take their keys: they answer it after a pause of 1.2 s (a `y` in "yet" once ended a handoff nobody meant to end).
  - On Hand back the agent gets the URL, the title and "the user changed the page" with the epoch moved, then re-reads.
- **Stop** (overlay, popup, or interrupting the turn in any Stitch client) detaches the debugger at once and cancels the turn's browser calls, as Esc does for a tool call today.
- **Tabs outside the group are the user's.** The agent never acts in them. A user's tab joins the group only when the user puts it there (dragging it into the group counts, and the extension reports it) or shares it (§6.4).
- **Two agents, one browser:** each session has its own group, and a tab belongs to one session. Two threads cannot act on one tab: the second gets "this tab is <thread>'s".

### 6.4 Pointing at things

- **"Share this tab with Stitch"** (toolbar popup, context menu, `⌥⇧S`): puts the tab in a session's group. It goes to the thread the user picks: the most recent ones are listed, plus "new thread".
- **"Ask Stitch about this…"** on a selection or an element (context menu, or a picker mode from the popup: hover to highlight, click to choose):
  - attaches to the thread as a `browser_context` part: the tab, its URL and title, the selected text, and the element as a ref in the tab's ref table (`ref_17`, `form "Billing address"`), together with a small screenshot of it;
  - from a new thread, it opens one.
- **Typed in a chat** ("this tab", "this form"): the `browser_context` tool returns, for every connected profile, the active tab of the focused window, the selection, and the element the user last right-clicked or picked, so "this" resolves without guessing. Reading the user's active tab outside a group is allowed only for its URL and title, and the shared selection or element. Reading the page itself requires sharing it.
- **Pointing from Stitch's clients:** a message composed while a tab is shared can include "the page as I see it" (a screenshot taken by the extension) as an attachment.

### 6.5 Agent-alone mode, and switching

- **Agent-alone:** the session's group lives in a **separate Chrome window** of the same profile, opened unfocused (`chrome.windows.create({focused: false})`) and minimisable. The agent works there with focus emulation.
  - The overlay still runs (for the user who looks).
  - User input there still pauses the agent: the rule does not change.
  - "Watching" is false (§8), so questions go to the user's devices.
- **Defaults:**
  - a session started from an attended chat on this machine is **cooperative**;
  - a detached thread's session, or one driven from another node (the phone), starts **agent-alone**.
- **Switching** moves the group between windows (`chrome.tabGroups.move`) and never reloads a page. The user switches with:
  - the overlay pill or the popup ("Bring here" / "Send to its own window");
  - `/browser coop|alone` in any client;
  - `stitch thread browser <id> coop|alone`.
  - The agent can only *ask* (`browser_mode {mode, reason}`). Moving into the user's window always needs the user, so an agent cannot put itself in front of them.

## 7. Tool surface

The `browser` group. Calls take `tab` (default: the session's current tab) and `browser` (default: `[browser] default`; fixed for the session once chosen). Every observation returns `epoch`.

| Stitch tool | What it does | Claude in Chrome | OpenAI | Playwright MCP |
|---|---|---|---|---|
| `browser_context` | connected profiles; the user's active tab (URL, title), the selection and the picked element; the session's tabs and their state (paused, held, epoch) | `tabs_context` | – | – |
| `browser_tabs` | new (url), select, close, list; `request` a user's tab (asks the user to share it) | `tabs_context`, `tabs_create` | multiple tabs | `browser_tabs` |
| `browser_navigate` | a URL, back, forward, reload | `navigate` | yes | `browser_navigate`, `_back` |
| `browser_snapshot` | the accessibility tree with refs, or the main text | `read_page`, `get_page_text` | text browser | `browser_snapshot` |
| `browser_find` | by role, name or text → refs | `find` | – | `browser_find` |
| `browser_screenshot` | viewport, full page, element; `region` zooms | `computer.screenshot`, `.zoom` | screenshots | `browser_take_screenshot` |
| `browser_act` | an ordered `actions` batch (click 1–3 with left, right or middle, hover, move, drag, type, key with `repeat`, hold_key, scroll, scroll_to, wait), by `ref` or `x,y`; a required `intent`; `observe` for the state after; stops at the first failure, a user's input, or a moved epoch | `computer`, `browser_batch` | `actions` array | click, type, hover, drag, press_key |
| `browser_fill` | several fields by ref | `form_input` | – | `browser_fill_form` |
| `browser_upload` | files from the thread's directory or blobs, into a file input or dropped at a point | `file_upload`, `upload_image` | – | `browser_file_upload` |
| `browser_downloads` | the session's downloads (`chrome.downloads`, filtered to its tabs) with paths | asks to download | agent: yes; Atlas: no | – |
| `browser_dialog` | answer a held dialog | – | – | `browser_handle_dialog` |
| `browser_wait` | a selector, text, URL, network idle or time | `computer.wait` | `wait` | `browser_wait_for` |
| `browser_handoff` | ask the user to take over (login, 2FA, CAPTCHA, payment, anything), and wait for Hand back | pauses at logins | takeover | – |
| `browser_mode` | ask to switch between cooperative and agent-alone | – | watch mode (sort of) | – |
| `browser_plan` | the domains and the approach, approved once | `update_plan` | checkpoints (Atlas) | – |
| `browser_eval` | JavaScript in the page, **off by default** in the user's Chrome | `javascript_tool` | none in the page | `browser_evaluate` |
| `browser_console`, `browser_network` | the console and requests, **off by default** in the user's Chrome | `read_console_messages`, `read_network_requests` | – | `browser_console_messages`, `_network_requests` |
| `browser_resize` | the viewport (agent-alone windows only; never the user's window) | `resize_window` | – | `browser_resize` |
| `browser_record` | GIF or WebM with the cursor and labels, plus a JSON trace | `gif_creator` | live view | tracing and video |

- **The `browser` group is on for desktop threads when an extension is connected,** and otherwise off. A `browser` skill teaches the workflow: context first, then snapshot, refs over coordinates, zoom before small clicks, batches, respecting pauses and epochs, and handoffs for logins and payments (open question 3).
- **Left out:**
  - cookie and storage tools (nothing reads the real profile's cookies);
  - request mocking;
  - CAPTCHA solving (always a handoff);
  - performance tooling (Chrome DevTools MCP can be added as an MCP server).

## 8. Safety in a real, shared profile

**The threat:**
- A page's content steers the agent into acting with the user's logins, or into leaking data: through URLs it navigates to, forms it posts, uploads, or the tabs it can read.
- A real profile raises the stakes.
- Cooperative use adds one more: the user's presence must not become a rubber stamp.
- Model-side defenses (Claude at about 1% with an adaptive attacker, below 0.08% on Anthropic's internal combined test; OpenAI's numbers are model-only) are not zero, and Stitch runs models without them. **The rules hold whatever the model does.**

### 8.1 Hard boundaries (the extension enforces them, whatever the node says)

- It attaches only to tabs in a Stitch group: ones the agent created, or ones the user shared or dragged in.
- It never reads cookies or storage, and has no `cookies` permission.
- It works only in the profiles the user listed in `[browser.chrome] profiles`; the extension's popup also has a per-profile "allow Stitch here" switch.
- It never handles `chrome://`, the Web Store, `file://` (except `file://` files the user shares), other extensions' pages, or `[browser] blocked` sites.
- **Stop always works:** it detaches locally, without the node.

### 8.2 Watching

A session is **watching** when all of these hold:
- it is cooperative;
- the agent's tab is the active tab of the focused Chrome window;
- the user has had input in Chrome within the last 60 s (`chrome.idle` and `windows.onFocusChanged`);
- the screen is not locked.

The extension reports changes as they happen. Watching is never inferred from the model's claims.

### 8.3 What asks, by mode

| Action | Watching | Not watching (cooperative but away, or agent-alone) |
|---|---|---|
| Reading the agent's tabs (snapshot, screenshot, find, context) | runs | runs |
| Clicks, typing, scrolling and navigation **within a granted site** | runs | runs |
| Same, on the current site when it is **not granted** | runs (the user sees it happen and can stop it) | asks |
| Navigating to **another site** (a new registrable domain) | asks in the page (one click: allow once, or always for this site) | asks |
| **The always-ask set** (below) | asks in the page, the element outlined | asks, with a screenshot, pushed to the phone |
| Sites on `[browser] sensitive` (banks, health, government, the user's admin consoles) | every changing action asks | **the agent does not act**: it asks the user to watch or take over |
| Passwords, OTPs, card numbers | **handoff**: the agent never types them | handoff (waits for the user) |
| CAPTCHAs, creating accounts | handoff | handoff |
| `browser_eval`, console, network | off unless `[browser.chrome] devtools = true`; then ask, per site | same |

- **Site grants** come from:
  - an "always" answer, kept per place (`[browser.places."<place>"] sites`);
  - `[browser.chrome] sites`;
  - `browser_plan` (for the thread's lifetime).
- **"Runs while watching" is the cooperative bargain:** the user sees the cursor and the labels and can stop or take over at any moment, so in-site steps flow, as in Claude's "allow for this site". What can leak or cannot be undone still asks even then: cross-site navigation and the always-ask set.

**The always-ask set** (every mode, yolo included):
1. paying or purchasing;
2. sending, posting or publishing (messages, mail, comments, merges);
3. account, security, sharing or permission changes, OAuth consent;
4. deleting;
5. uploading a local file;
6. downloading an executable or archive;
7. requesting one of the user's tabs.

It is detected by:
- **the action's required `intent`** (`read`, `navigate`, `input`, `submit`, `purchase`, `send`, `delete`, `account`): a consequential intent always asks;
- **heuristics on the target:** `autocomplete="cc-*"`, a checkout form, button text in the user's languages, password or OTP fields, and the site lists;
- **a mismatch** between the declared intent and what the heuristics see: it asks.

### 8.4 Where questions show

- **Watching:** in the page, as an overlay bar ("Stitch wants to submit 'Place order' on shop.example: Allow once · Always on this site · Deny"). It is also written as the usual `question` record, so the transcript has it and any client can answer it. The first answer wins.
- **Not watching:** a `question` record, pushed to the phone. It carries a screenshot with the target outlined. This is new: a `question` may hold an image (a small records change).
- **Approvals given while watching say so** in the record ("approved in the page").

### 8.5 Content, secrets and memory

- Page results are fenced as untrusted (`<page origin="…">…</page>`), and the skill and prompt say what is inside is data, never tasks.
- Password values are masked in snapshots. Screenshots of logged-in pages go to the model provider and sync with the thread to all nodes (decided, §12).
- Uploads come only from the thread's directory or its blobs. `.env`, `*.pem`, `~/.ssh` and similar are refused.
- **No unasked engram recall in turns that used the user's Chrome,** and their page results are not observed into memory, as ChatGPT agent turns memory off (default, §12).
- The extension talks only to its native host: no remote server, no account. The host manifest admits only the extension's pinned ID. The node's socket is the user's own (owner-only, as today).

## 9. Otter

- Phases B0 to B4 need nothing from Otter.
- **Later, `Browser.*` host shards:** a `Browser.Tab` object, the browser registry as a service, cleanup guards that release tabs, and cancellation that owns the transport. These are H1, H3, H4 and H6 (Track A1), plus host shards with declared effects (Track C). They let skills' tools and **recorded workflows (B6) become checked Otter scripts**.
- **Cooperative rules hold for scripts too:** pauses and epochs surface as errors a script must handle, and approvals are asked through a host service from inside a shard (an `AsyncShard` waiting on it). Requests for Otter's `host-requirements.md`, worded without naming the host:
  - *a host shard may suspend on a host service that asks a human*;
  - *grants per effect and per target*.
- Screenshots are blob references, so H2 (image values) stays deferred.

## 10. Risks

- **The infobar.** Chrome's "started debugging this browser" bar is unavoidable with `chrome.debugger`, and Claude in Chrome lives with it. Mitigation: attach only while acting, detach when idle.
- **Painting in background tabs** (B0: fresh frames on Chrome 153 with no workaround, §15.3): screenshots of tabs not in front. Claude needed workarounds ("hidden tab draw"). B0 must prove it, or cooperative mode falls back to "the agent's tab is in front when the agent screenshots it", which steals the user's focus.
- **Telling user input from the agent's:** CDP events are trusted too. B0 found dispatch windows fail under overlap and replaced them with a mark on the agent's own events (§15.5).
- **Web Store review** of a `debugger` + `<all_urls>` extension can take time or be refused. Sideloading works meanwhile.
- **The 144 consent flow is new and may change.** It is the fallback, behind a transport.
- **Other extensions' frames block the debugger.** Chrome refuses `chrome.debugger.attach` on a tab holding a frame of another extension ("Cannot access a chrome-extension:// URL of different extension"), and detaches when one comes in. Password managers and privacy extensions inject such frames once they see login or email fields (found live 2026-10-19: Proton Pass's `protonpass-root-*` elements, closed shadow roots holding `chrome-extension://…/dropdown.html` and `notification.html`; DuckDuckGo Privacy Essentials does the same), so clicks broke on a form right after `browser_fill` focused its email field. What the extension does (§16, "Other extensions' frames").
- **The accessibility tree misses canvas apps** (Docs, Figma): screenshots and coordinates.
- **Heuristics in the always-ask set miss cases:** layered with intent, site grants and cross-site asks; the hard boundaries are the extension's.

## 11. Phases

In order. The live view and takeover from the phone come straight after the first cooperative sessions (decided 2026-10-17).

| Phase | Contents |
|---|---|
| **B0 spike, both mechanisms, on this Mac's Chrome 155** | (1) **Extension:** a minimal unpacked MV3 extension with a pinned key, plus `stitch browser-host` relaying to a node stub. It creates a coloured tab group, attaches `chrome.debugger`, takes a snapshot and screenshot of a background tab (focus emulation), clicks and types by ref, and draws a glow and a Stop button. **Minimal cooperative loop:** the user types in the agent's tab mid-batch, the agent pauses, the epoch moves, the next action is refused with what changed, and after the idle time it resumes from a fresh snapshot. Measure the infobar's behaviour and painting in background tabs; check a second profile. (2) **Consent attach:** switch it on, connect with `DevToolsActivePort`, count dialogs across a node restart and a Chrome restart, check whether the toggle persists, reach the other profiles, and run the same loop with an injected overlay. (3) **Snapshot:** CDP accessibility versus a content-script tree, on 6 sites. Findings are written into this document, and the attach decision is confirmed or revised. Notes for the run: Chrome 137+ ignores `--load-extension` in branded Chrome, so automated runs use Chrome for Testing (145 is in `~/.cache/puppeteer`) and a throwaway `--user-data-dir`, whose `NativeMessagingHosts/` holds the host manifest. Branded Chrome needs the user to click "Load unpacked". Never use the user's own profiles |
| **B1 cooperative sessions in the user's Chrome** | The engine (sessions, tabs, refs, snapshot, find, screenshot and zoom, act batches, fill, wait, dialog, upload, downloads, context, handoff, mode); the `Extension` transport; the extension proper (relay, groups, badges, overlay with cursor and labels, Stop / Take over / Hand back, the input watcher, share-this-tab, picker, context menu, popup); epochs and arbitration (§6.2–6.3); agent-alone windows and switching; detached threads in agent-alone mode; the profile picker (§12.1); `stitch browser install` (the extension's files and host manifests for Chrome, Brave and Edge, load-unpacked instructions; the headless download moved to `stitch browser install headless`); the safety model of §8 (hard boundaries, watching, the table, the always-ask set with intent, in-page questions, `question` with an image, site grants, `browser_plan`, the fence, memory off); the `browser` skill; tests with fixture pages in a real Chrome loading the unpacked extension (Chrome for Testing accepts `--load-extension`), the injection page included |
| **B2 the phone: across nodes, live view and takeover** | The live channel's first part, pulled forward (design §5, milestone 6): an iroh stream per watched tab carrying `Page.startScreencast` frames to the app (and to `stitch app` on desktops), touches and keys back as `Input.*`, Take over / Hand back from the app, nothing observed by the model during a takeover. Share browsers in `capabilities`; calls over `Ask` with images; phone threads drive the Mac's Chrome (agent-alone by default, questions to the phone); a session the user picks up at the Mac or on the phone's live view turns cooperative |
| **B3 consent-attach fallback** | The `Cdp` transport over the 144 flow, overlay and watcher injected into an isolated world, epochs and pauses the same; used when no extension is connected (`consent_attach = true`) |
| **B4 Web Store** | An unlisted listing, the privacy policy, the review; auto-updates; version handshake between the extension and the node (as `Hello`) |
| **B5 secondary backends** | `ephemeral` sessions on today's headless Chromium (the web tools' launcher split); `managed` profiles for an always-on box (gamer); `connect` for CDP addresses |
| **B6 recording and replay** | `browser_record` (GIF and WebM with the overlay cursor, a JSON trace); later, traces as Otter scripts (after Track A1 and host shards) |
| **B7 optional** | The `auto` mode (a classifier per action), password-manager fill, natural-language `find` through a small model, a side panel with the thread in Chrome |

## 12. Decisions (2026-10-17)

Earlier, the user's direction settled where logged-in work happens (**your Chrome**), how to reach it (**Stitch's extension first**, the consent attach as fallback), and which node holds the profile (**the machine where you browse, mbp**; gamer only with a managed profile, B5). Then the user answered the open questions:

1. **Profiles: the user picks, with no fixed list** (§12.1). Nothing is allowed until a profile is enabled.
2. **Install: sideloaded for now, by `stitch browser install`,** which installs the extension's files and the native-messaging host. The Web Store listing comes later (B4).
3. **Resume after the user's input: by itself, about 4 s after they go idle, from a fresh read.**
4. **Detached threads may use the user's Chrome while they are away, in agent-alone mode.**
5. **Screenshots of logged-in pages may sync to all nodes,** as other tool images do.
6. **The live channel is pulled forward**, so the user can watch and take over from the phone: phase B2, straight after B1.

For the rest, the plan's recommendation is **the default** until the user says otherwise:

7. **The `browser` group** is on for desktop threads while an extension is connected (default).
8. **The watching bargain:** while the user watches, in-site clicks and typing run without asking, even on a site not yet granted. "Watching" ends after 60 s without input in Chrome (default).
9. **Memory:** off in turns that used the user's Chrome: no unasked recall, and nothing observed (default).
10. **Tabs at the end of a session** are left open and ungrouped (default).
11. **Other browsers:** `stitch browser install` writes host manifests for every Chromium browser found (Chrome, Brave, Edge), so Brave works from the start (default).
12. **Smaller:** the `auto` classifier mode and natural-language `find` through a small model are later (B7), and `find` matches locally until then. Recording is on demand only (default).

### 12.1 The profile picker (decided with point 1)

- **No profile can be used until the user enables it.** A newly connected profile registers with the node as `disabled`, and every call on it fails with "Stitch is not enabled in this Chrome profile: open the Stitch extension there and switch it on".
- **Enabling is a click in that profile's extension popup** ("Allow Stitch in this profile", with the profile's name, which the user can edit, and the browser). It is never a command: agents run `stitch` commands in their own shells, and enabling must take a human at that browser. The extension keeps the switch in `chrome.storage.local` and enforces it itself: while it is off, it attaches to nothing and relays nothing but the hello.
- **`stitch browser profiles`** lists every profile that has connected (browser, name, enabled or not, sessions running), and `stitch browser profiles disable <name>` turns one off. Disabling is safe in either direction, so the command does it; the extension shows it as off.
- **`/browser` in the TUI and the app's settings** list the same and can disable. To enable, they say where to click.
- **Several profiles enabled:** a session picks one with `browser: "chrome:<name>"`. Without one, the default is the only enabled profile, or the user is asked once per thread.

## 13. Open questions

None blocking. Points 7 to 12 of §12 stand as defaults and can be changed at any time.

## 14. What changes in design.md (done with the decisions)

- **§2.3:** "interactive browser sessions" moves from later work to a new **§2.18 Browser control**:
  - the user's Chrome through Stitch's extension (and the consent fallback);
  - cooperative and agent-alone sessions;
  - the engine and its transports;
  - §8's safety model.
- The web tools' bullet says they stay stateless and logged out.
- **§2.1:** a new local client, `stitch browser-host`: a native messaging relay with a `BrowserLink` stream on the node's socket, versioned like `Hello`. The repository gains `apps/chrome/` (the extension).
- **§2.14:**
  - the tool group `browser`;
  - "tools on another node" extends to shared built-in groups (`[browser] share`), browsers in `capabilities`;
  - a new context part, `browser_context` (what the user pointed at).
- **§2.3 approvals and records.md:**
  - a `question` may carry an image, and may be answered in the page;
  - site grants (`[browser.places]`) sit beside `[otter.places]` and the MCP grants;
  - the always-ask set is the first rule that yolo does not override.
- **§2.9:** memory is not recalled or observed in turns that used the user's Chrome.
- **§2.17:** "grants per effect rather than per tool" gets a concrete case.
- **§3:** H1, H3, H4 and H6 gain the browser as a user; host shards (Track C) list `Browser.*`; H2 stays deferred.
- **§5:**
  - B0, B1 and B3–B5 go into milestone 3 (Stitch as the root: tools);
  - B2 pulls milestone 6's live channel forward in part;
  - B6's replay goes into milestone 5.
- **README:** a "Browser control" section (installing the extension, cooperative use, Stop and Take over), and `[browser]` in the setup manual.

Recorded in design.md §2.18 (2026-10-17), with the pointer in §2.3 and the note in §5's milestone 6. The other items above (§2.1, §2.9, §2.14, records.md, README) are written as each phase builds them.

## 15. B0 findings (2026-10-18)

Run on gamer (Linux, CachyOS) rather than the Mac: branded **Google Chrome 153.0.8010.47** and **Chrome for Testing 153.0.8010.12** (Playwright's `chromium-1243`), each on a headless X server (Xvfb `:99`) with **throwaway `--user-data-dir`s only**. The user's simulated hand is XTEST (`human.py`): real input through the window system, as a mouse and keyboard give. The spike is kept in `tools/browser-b0/` (its README says how to run it). **Verdict: the extension path holds; the plan stands,** with the changes in §15.8.

### 15.1 Loading the extension on Linux

| How | Branded Chrome 153 | Chrome for Testing 153 |
|---|---|---|
| `--load-extension=<dir>` | **ignored** (no error, no extension) | works, in every profile of that data directory |
| `--disable-features=DisableLoadExtensionCommandLineSwitch` | ignored too | – |
| CDP `Extensions.loadUnpacked` over `--remote-debugging-pipe` with `--enable-unsafe-extension-debugging` | works, and **persists**: after a restart without flags the extension loads and connects | works |
| the same on the **default** data directory | refused: "DevTools remote debugging requires a non-default data directory" (port and pipe alike) | – |
| external extension JSON (`<id>.json` with `external_crx`, a packed `.crx`) | needs root (`/opt/google/chrome/extensions/`); not tried | works from `<exe dir>/extensions/`: installed enabled, no prompt, location "external pref" |
| policy (`ExtensionInstallForcelist`, self-hosted update XML) | needs root (`/etc/opt/chrome/policies/managed/`); not tried | – |
| "Load unpacked" by hand (developer mode) | the path for the user's own profile | – |

- So for the user's real profile, **"Load unpacked" is the only path without root**, as the plan assumed. `stitch browser install` writes the files and says where to click. On Linux a root install (`/opt/google/chrome/extensions/<id>.json` plus a packed `.crx`) would make it one step with no developer mode; worth offering later as `--system`.
- **Tests use Chrome for Testing with `--load-extension` and a fresh profile each run.**
- **A gotcha for development:** an unpacked extension keeps running its **old service worker** after its files change and Chrome restarts with the same profile (content scripts are re-read; `sw.js` is not, even with a version bump). `chrome.runtime.reload()` left it disabled (`disable_reasons` 1<<24) in Chrome for Testing. Fresh profiles for tests; for the user, the extension's update path needs a check in B1 (the "reload" in `chrome://extensions` works). Since the fix for issue 2026-11-01T141210Z-gamer-i65v the node no longer leaves this silent: `install` stamps `sw.js` with a build id (a hash of the extension's files; `const BUILD = "dev"` when loaded straight from `apps/chrome`), the node rewrites an existing install's files at start (`install::refresh`), and the hello's `build` is compared on connect. A missing or different build (not `"dev"`) marks the link stale: it keeps working, but its tool errors and `browser_context` say to press reload on Stitch's card in `chrome://extensions`.

### 15.2 Native messaging and reconnection

- The host manifest in `<user-data-dir>/NativeMessagingHosts/` is honoured by both builds (so tests need no files in `~/.config`).
- Chrome starts one host process **per profile** (`connectNative` in each profile's service worker); each profile keeps its own random ID in `chrome.storage.local`, which survives restarts.
- **Chrome restart:** the service worker's `connectNative` at start reaches the node 4–40 ms after the worker starts; the profile ID is the same.
- **Node restart:** the host reconnects to the node's socket (polled once a second in the spike) and sends the hello again; no click, nothing in Chrome changes.
- **The service worker stays alive with the native port open:** requests after 40 s, 120 s and 330 s of silence answered in 4–5 ms, the port never closed.
- Overhead: an `attach` takes 3 ms, a `detach` 4 ms; a CDP round trip through extension, host and node is a few ms.

### 15.3 Background tabs

With the user's tab in front and the agent's tab behind it in the same window (both builds):
- `document.visibilityState` is `hidden` and `hasFocus()` false; `Emulation.setFocusEmulationEnabled` makes them `visible` / `true`.
- **`Page.captureScreenshot` of the background tab returns fresh frames**, with or without focus emulation: 35–90 ms, and 190–200 ms after the tab had been hidden for 8 s; the page's 100 ms clock and a DOM change made while hidden both show. No "hidden tab draw" trick was needed on 153.
- `Page.startScreencast` on a background tab delivers frames (33–36 in 3 s), which is what B2's live view needs.
- Clicks and typing by CDP `Input.*` land in the background tab; the user's tab stays active, nothing is activated.

### 15.4 The debugging infobar

- Branded Chrome shows **"'<extension name>' started debugging this browser"** with Cancel and ×, within 0.5 s of the first attach. It is **browser-wide**: it shows over the user's own tabs too, and moves every page down by its height (about 37 px), which changes `window.innerHeight` (coordinates must be taken after it shows).
- It **stays about 5 s after the last detach** (measured 4.8–5.4 s), so detaching between batches only hides it when the agent is idle for longer than that.
- **Cancel and × both detach every tab** (`onDetach` reason `canceled_by_user`). The engine must treat that as the user pressing Stop. The next attach shows the bar again.
- **Chrome for Testing shows no such bar** (only its "for automated testing" banner), so tests cannot measure the bar; layout tests must not assume it.

### 15.5 The cooperative loop (extension)

The fixture: a form and a 100 ms clock. The agent fills Name, Email and Notes by ref in its own tab group; at 1 s the simulated user clicks Notes and types 19 characters at 70 ms a key.
- The agent stopped **mid-word** ("ada@" of the email, 4 characters in), **24 ms** after the user's first event; the next step was **refused** with the epoch moved and what the user did (`pointerdown` on `textarea#notes`); the agent waited for **4 s without input**, took a fresh snapshot (which showed the user's text), skipped the field the user had worked in, and finished. Final values: the agent's name and email, the user's notes. All 20 user events reported (a click and 19 keys), none of the agent's.
- **Attribution by dispatch window does not work:** with the agent typing steadily (a key every ~10 ms) and a fast typist (40 keys, 25 ms apart) at the same time, the window was always open and **0 of 40** of the user's keys were seen. It also leaked once in a quiet run (the late "close" of one command closing the next command's window).
- **What works instead: the agent marks its own input.** CDP `Input.dispatchMouseEvent` / `dispatchKeyEvent` take a `timestamp`; given `timestamp: 1` (1970), the page sees **`event.timeStamp === 0`**, which real input never has. With the mark: **40 of 40** user keys seen during the same overlap, **0** agent events misreported. No window, no timing, no messages to the content script per event. (Derived events such as `input` carry their own time, so the watcher only listens to raw ones: `pointerdown`, `keydown`, `wheel`, `paste`.)
- The overlay (shadow DOM: glow, cursor, label, pill with Stop) draws and is hidden for screenshots.
- **Agent-alone:** `chrome.windows.create({focused: false})` makes the window; screenshots and input work there; `chrome.tabGroups.move` moves a group between windows. Whether an unfocused window takes focus depends on the window manager: Xvfb has none, so this is untested (to check on the Mac and on a Wayland desktop).
- **A second profile** in the same Chrome (`--profile-directory="Profile 2"`): it starts its own host and registers with its own ID; both drive independently.

### 15.6 The consent attach (branded Chrome 153)

- `chrome://inspect/#remote-debugging` → "Allow remote debugging for this browser instance": a **persistent** switch (in `Local State`, `remote_debugging.user-enabled`); after a restart Chrome listens again on 127.0.0.1:9222 and writes `DevToolsActivePort`.
- The HTTP endpoints (`/json/version`, `/json/list`) answer nothing; only the browser WebSocket.
- **Every WebSocket connection asks:** a modal "Allow remote debugging?" (Turn off in settings · Cancel · Allow) in the last focused window. The handshake is held until the click: **no timeout seen in over 2 minutes**; Cancel fails the handshake (close 1006). A second connection a second after the first asks again. So: one click per node restart, and per Chrome restart, as expected.
- While connected, the whole browser shows "Chrome is being controlled by automated test software".
- **One connection reaches every open profile** (two browser contexts' pages, and other extensions' service workers), so scoping would be Stitch's alone.
- **The same cooperative loop works over it:** a background target (`Target.createTarget {background: true}`), the watcher and a glow injected into an isolated world (`Page.addScriptToEvaluateOnNewDocument {worldName}`), reports through `Runtime.addBinding {executionContextName}`, the agent's input marked with `timestamp: 1`. Result: paused **≤ 60 ms** after the user's first event, refused once, resumed 4 s after the last input, 20 of 20 user events, final values as with the extension. There is no tab group, toolbar or "this tab" button, as planned.
- The Allow dialog could be clicked by synthetic X input (XTEST) on this headless X server. It is a consent for whoever sits at the display, not a defence against local software.

### 15.7 Snapshots: CDP's tree versus a walk in the page

The same six pages, the CDP full accessibility tree (filtered to interesting roles, relayed through the extension) against a DOM walk run in the page (as a content script would):

| Page | CDP `Accessibility.getFullAXTree` | DOM walk in the page |
|---|---|---|
| Wikipedia "Ada Lovelace" | 15,076 nodes → 1,458 refs, 49 k chars, **692 ms** | 6,116 elements → 1,464 lines, 33 k chars, **5 ms** |
| github.com/microsoft/playwright | 3,684 → 323, 11 k, 193 ms | 2,815 → 335, 8 k, 3 ms |
| news.ycombinator.com | 1,634 → 231, 7 k, 91 ms | 813 → 231, 5 k, 3 ms |
| amazon.com | 1,604 → 258, 10 k, 188 ms | 1,916 → 301, 10 k, 2 ms |
| MDN HTML | 1,117 → 155, 5 k, 75 ms | 1,515 → 419, 6 k, 8 ms |
| bbc.com/news | 1,685 → 178, 17 k, 87 ms | 1,285 → 178, 9 k, 2 ms |

- The counts of what matters agree; the CDP way costs 30–150 times as long, almost all in shipping every node across native messaging (Chrome's 1 MB limit applies to messages *to* the extension, not from it, so it works, but slowly).
- **Decision for B1: the snapshot is built in the page,** by a script in an isolated world: the content script with the extension, `addScriptToEvaluateOnNewDocument {worldName}` over the consent attach. Refs live in the page (elements in a map, invalid after navigation), and the script returns coordinates for input, so the CDP side stays `Input.*`, screenshots and navigation. This is what Claude in Chrome does. CDP's tree stays a fallback for frames the script cannot reach.

### 15.8 What changes in the plan

- §5.2: refs are kept in the page by the snapshot script, not bound to `backendDOMNodeId` (§15.7).
- §6.3: the watcher tells the agent's input by its mark (`timestamp: 1` → `event.timeStamp === 0`), not by dispatch windows (§15.5). The node no longer tells the extension when a window opens.
- §6.1, §10: the infobar lingers 5 s after the last detach and is browser-wide; Cancel or × on it is a Stop (§15.4).
- §10: painting in background tabs is not a risk on Chrome 153 (§15.3).
- §11 B0 notes: Chrome for Testing is in `~/.cache/ms-playwright` on gamer; tests load the extension with `--load-extension` and a fresh profile (§15.1).
- **Still to check on the Mac:** the same with Chrome 155 on macOS, the infobar and focus behaviour under a real window manager, and `windows.create({focused:false})` not stealing focus.

## 16. B1 progress (2026-10-18, branch `browser-tool`)

**Built and tested** (unit tests, and `crates/stitch-cli/tests/browser.rs`: a headless Chrome for Testing with the extension, a throwaway node, every tool through `stitch browser call`; it skips where no Chrome for Testing is found):
- **The extension** (`apps/chrome/`, ID `laglcmonaljgkdmikgohcogdmjihlhel`, plain JS): the service worker relays CDP per tab and runs sessions as tab groups (cooperative: in the user's last focused window, tabs opened in the background; agent-alone: an unfocused window of its own; switching moves the group). It attaches only to its sessions' tabs and tabs the user dragged into a session's group or shared from the popup, refuses `chrome://`, extension pages, the Web Store and `file://`, relays no `Browser`, `Target`, `Storage` or cookie commands, detaches after 20 s idle, and does nothing but say hello until the profile is enabled in the popup. Cancel or × on Chrome's bar is a Stop. The page script builds snapshots with refs (§15.7), resolves refs to points (and says what covers one), fills fields as frameworks notice, draws the overlay (glow, cursor, label, Stop, Take over / Hand back), and reports the user's raw input, skipping the agent's marked events (§15.5).
- **`stitch browser-host`**: the relay (Chrome's framing ↔ `BrowserLink` / `BrowserMsg` on the node's socket, local protocol 29); it starts the node, reconnects when it restarts, exits on a version change, when the node refuses the link, or when `stitch` was installed again since it started, so Chrome starts the new binary. (A test in `api` pins the protocol's shape to its number, so a change of shape cannot keep the version.)
- **The registry**: links per profile, profiles remembered in `<data>/browser/profiles.json`, `stitch browser profiles [disable <name>]` (a disable for a profile away is applied when it connects).
- **The engine and the tools**: `browser_context`, `browser_tabs`, `browser_navigate`, `browser_snapshot`, `browser_find`, `browser_screenshot` (viewport, ref, region, full page; CSS pixels, scaled to at most 1568 px with the mapping said), `browser_act` (click 1–3, right, hover, drag, type with `replace`, key chords with `repeat`, scroll, scroll_to, wait), `browser_fill`, `browser_wait`, `browser_dialog`, `browser_handoff`, `browser_mode`. Sessions per thread with a stable ID, so after a node restart the thread finds its group and tabs again. Epochs, pause mid-batch (and mid-typing) on the user's input, waiting for 4 s of quiet, refusal with what changed and a fresh read, Stop ending the turn's browser use, Take over and handoff holding a tab.
- **Safety** (§8): the declared intent is required on `browser_act`; the whole batch is classified before any of it runs (intent, the targets' look, secrets) and asks once; another site, the consequential set and look-alikes always ask (yolo included); in-site actions run when the site is granted (`[browser] sites`, or approved in this session) or the user is watching, else ask; sensitive sites refuse unwatched; blocked sites never open; password, code and card fields are never typed. Page text is fenced.
- **`stitch browser install`** (the extension and host manifests for Chrome, Chromium, Brave and Edge, or `--profile-dir` for a test profile), `install headless` for the old download, and the `browser` skill.

Checked by hand on gamer (Chrome for Testing on Xvfb, the user simulated with XTEST): the agent works in a background tab of its own group while the user's tab stays in front; a user click and typing during a 290-character `type` paused it after part of the text, the user's text stayed, and the next read waited for quiet. Stop in the page ended a running `browser_wait` at once (a later turn's call reopens the session and brings the overlay back); Take over refused reads, and Hand back ended a waiting `browser_handoff` with the change reported.

**Added the same day (second pass):**
- **Questions where the user is** (§8.4): while they watch, a bar in the page (Allow once · Always on this site · Deny, the target outlined) besides the usual question; the first answer wins and is written as an answer saying "approved in the page". Unwatched, the `question` record carries a screenshot of the tab with the target framed in red (`tab.png`, an attachment), so the phone shows what is asked. "Always" on a site question (another site, a site not granted) keeps the site for the thread's place in `[browser.places."<place>"] sites`; the always-ask set offers no "always".
- **The group is named after the thread's title** (renamed when the thread gets one). A detached thread's session ends when its turn reports; any session after 30 minutes without a call. Tabs stay open, ungrouped.
- **Memory:** the first browser call of a thread takes it out of memory (no unasked recall, nothing observed), unless the user switched its memory themselves; the result says so, and `/memory on` brings it back. Per thread rather than per turn: simpler, and stricter.
- **Pointing:** "Ask Stitch about this…" in the context menu (page, selection, link, image, field) and the popup's picker send the element (as a snapshot line with its ref), the selection, the link or the image; `browser_context` lists the last five. "Share this tab" with no session offers the tab; `browser_tabs` action `share` takes a tab the user offered (the extension adopts no other). A tab loaded before the extension (installed, reloaded or updated) has no content script: the service worker injects it (`scripting`) the first time it messages the page, so such a tab can be shared and read without reloading it. `text: true` reads the `main`, `[role=main]` or `article` with the most text (the first can be an empty card), or the body when none has 500 characters.
- **`browser_upload`** (files from the thread's directory into a file input by ref, through `DOM.setFileInputFiles` on a staged copy held until the session ends, so the original may be deleted; always asks; keys, `.env`, `~/.ssh` and the like refused), **`browser_downloads`** (the session's downloads with paths; executables and archives paused until `keep`, which asks), **`browser_plan`** (sites and approach, approved once: the sites open without asking for the rest of the session).
- **Same-origin iframes** are read, found and acted in as part of the page (coordinates through the frames' offsets); the input watcher runs in every frame. Cross-origin frames show as `iframe … (another site's frame: not read; screenshot it)`.
- Tests: `crates/stitch-node/tests/browser.rs` (a fake extension on the node's socket and a scripted model: questions in the page and with a screenshot, always per place, memory off, pointing and taking a tab) and the real-Chrome test (iframes, upload refusals, a download).

**Other extensions' frames** (2026-10-19, §10):
- **Recovery:** when the attach is refused for another extension's frame, the service worker finds those frames in every frame of the tab, inside shadow roots too (closed ones through `chrome.dom.openOrClosedShadowRoot`), removes them and attaches once more. A command whose debugger was detached under it (the frame came back) attaches again once.
- **The reason, when still refused:** "another extension's frame blocks control of this tab: extension <id> (<name>, for a few known ones: the name needs the `management` permission otherwise) …; ask the user to turn that extension off for this site", without the DevTools hint. The tab is then marked refused for 15 s (or until it navigates), so the CDP calls of one action are not split between the debugger and the fallback.
- **Fallback:** `Input.dispatchMouseEvent`, `dispatchKeyEvent` and `insertText` are played by the page script as DOM events (pointer and mouse events with focus on press and a click on release, wheel scrolling, keys with the defaults the agent uses: text, Enter submitting or clicking, Space, Backspace, Delete, Tab, Ctrl/Cmd+A, arrows in a select, paging). They are untrusted (`isTrusted` false), so the input watcher ignores them and they never move the epoch; some pages ignore them too. Password, code and card fields are refused there as well, and a held tab takes none. `Page.captureScreenshot` becomes `chrome.tabs.captureVisibleTab`, cut and scaled to the clip, only when the tab is in front of an unminimised window (else an error saying so); `Page.getLayoutMetrics`, `Page.navigate` and `Page.reload` go through the page script and `chrome.tabs`; the rest (history, dialogs, uploads) fail with the reason. Such results carry `stitch_fallback`, which `browser_act` and `browser_screenshot` add to their result as a note.
- **Snapshots and `browser_find` walk shadow roots,** closed ones too, and list another extension's frame as such.

**Still not done:**
- the other-extension fallback is not covered by an automated test (it needs a second extension that injects a frame);
- cross-origin iframes (content scripts in every frame with their own ref tables, or CDP's tree for them), and a pointed element inside a frame;
- uploads from blobs (`img:` IDs), and the file chooser a custom button opens (`Page.setInterceptFileChooserDialog`);
- the phone and the TUI showing a question's screenshot (it is in the record; the clients do not display attachments of questions yet);
- the user-input part of the automated test (it needs real input: done by hand with XTEST);
- trying it in the user's real Chrome on the Mac (Load unpacked; the infobar and focus under a real window manager).
