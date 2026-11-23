#!/usr/bin/env bash
# B0 spike: start a Chrome on the Xvfb display :99 with a NEW throwaway
# profile in /tmp/stitch-b0/<profile>, the spike extension loaded and the
# native host manifest in the profile's own NativeMessagingHosts/.
#   launch.sh [cft|branded] [user-data-dir name] [extra chrome flags…]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
kind="${1:-cft}"; name="${2:-profA}"; shift 2 || true
case "$kind" in
  cft) exe="$HOME/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome" ;;
  branded) exe=/usr/bin/google-chrome-stable ;;
esac
udd="/tmp/stitch-b0/$name"
mkdir -p "$udd/NativeMessagingHosts"
chmod +x "$here/host.py"
cat >"$udd/NativeMessagingHosts/com.fragcolor.stitch.browser.json" <<EOF
{
  "name": "com.fragcolor.stitch.browser",
  "description": "Stitch B0 spike host",
  "path": "$here/host.py",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://agkhlnobmegpikaaficaldkpmlhkidmf/"]
}
EOF
# Never the user's desktop: X only, on the spike's Xvfb.
unset WAYLAND_DISPLAY
# No session bus: file choosers would otherwise go through the desktop's
# xdg-desktop-portal and open on the user's screen.
export DBUS_SESSION_BUS_ADDRESS=disabled: GTK_USE_PORTAL=0
DISPLAY=:99 exec "$exe" --ozone-platform=x11 --user-data-dir="$udd" --no-first-run --no-default-browser-check \
  --window-position=0,0 --window-size=1400,950 --load-extension="$here/ext" \
  --disable-search-engine-choice-screen "$@" about:blank
