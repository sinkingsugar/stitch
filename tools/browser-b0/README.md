# B0 spike: driving Chrome through an extension and through the consent attach

Kept as a reference for docs/browser-tool.md §15 (the findings). Not part of
the build. Linux, run against a headless X server so nothing touches the
user's desktop, and only throwaway profiles under `/tmp/stitch-b0/`.

- `ext/`: the minimal MV3 extension (pinned key → ID
  `agkhlnobmegpikaaficaldkpmlhkidmf`): `sw.js` relays requests from the node
  stub to `chrome.debugger`, `chrome.tabs`, `chrome.tabGroups`,
  `chrome.windows`; `content.js` draws the overlay and reports the user's
  input (anything trusted whose `timeStamp` is not 0).
- `host.py`: the native messaging host (stdio ↔ the stub's Unix socket,
  reconnecting).
- `driver.mjs`: the node stub and the scenarios (`bg`, `coop`, `race`,
  `mark`, `look`, `infobar`, `infobar2`, `cancelbar`, `closebar`, `alone`,
  `idle`, `snap`, `wait N`).
- `consent.mjs`, `consent-coop.mjs`: the Chrome 144+ consent attach, and the
  cooperative loop over it.
- `pipe-load.mjs`: loads the extension with CDP `Extensions.loadUnpacked`
  over `--remote-debugging-pipe` (works on branded Chrome, persists).
- `human.py`: the simulated user (XTEST clicks and typing, `python-xlib`).
- `launch.sh [cft|branded] <profile> [flags…]`: a Chrome on `:99` with the
  extension and the host manifest in the profile's `NativeMessagingHosts/`.
- `cdp-eval.mjs`: evaluates in a target over a debugging port (to look into
  the service worker).

```sh
Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp &
./launch.sh cft profA &          # Chrome for Testing 153 (Playwright's chromium-1243)
node driver.mjs coop             # the cooperative loop
```
