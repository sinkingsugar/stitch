#!/usr/bin/env bash
# Atelier's rebuild end to end on a node of its own (design §2.23): a
# temporary data directory, a mock model (mock_model.py, here), the app
# copied with its data folder in the temporary directory. The app is shown,
# its threads, build and commands granted (shown first, then by hash), the
# image built; then, as the page would, the recipe is written into the data
# folder (stitch.files), a thread asked for a shorter hem: it edits the
# recipe, runs rebuild_garment in the container, and the preview comes back
# as an image of its reply; the GLB is read through stitch.files.
#   tools/app-containers/atelier_e2e.sh <Atelier's app dir> [stitch binary]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
src="${1:?the Atelier app directory (sl-fashion-studio/apps/studio)}"
stitch="${2:-$repo/target/debug/stitch}"
work="$(mktemp -d /tmp/atelier-e2e-XXXXXX)"
export STITCH_DATA="$work/data"
mock_port=7288
apps_port=7287
mkdir -p "$STITCH_DATA"
app="$work/app"
cp -r "$src" "$app"
rm -rf "$app/assets"
# The data folder in the temporary directory, not the user's ~/Apps.
python3 - "$app/stitch-app.json" "$work/Atelier" <<'EOF'
import json, sys
p, folder = sys.argv[1], sys.argv[2]
m = json.load(open(p))
m["threads"]["data"]["folder"] = folder
json.dump(m, open(p, "w"), indent=2)
EOF
python3 "$here/mock_model.py" "$mock_port" "$app" Atelier >"$work/mock.log" 2>&1 &
mock=$!
# The test node's images are its own (labelled with its data directory):
# removed with it.
cleanup() {
  "$stitch" app storage --rm Atelier >/dev/null 2>&1 || true
  kill $mock 2>/dev/null || true
  "$stitch" node stop >/dev/null 2>&1 || true
}
trap cleanup EXIT

"$stitch" setup --name e2e --local >/dev/null
cat >>"$STITCH_DATA/config.toml" <<EOF

sync_interval_secs = 0

[apps]
port = $apps_port

[providers.anthropic]
kind = "anthropic"
base_url = "http://127.0.0.1:$mock_port"
EOF
echo sk-mock | "$stitch" key set anthropic --file >/dev/null

"$stitch" chat "Show the app in $app as Atelier" >/dev/null 2>&1
thread="$("$stitch" threads 2>/dev/null | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{6}Z-e2e-[a-z0-9]+' | head -1)"
echo "conversation: $thread"
shown="$("$stitch" app allow-threads Atelier --thread "$thread")"
needs="$(echo "$shown" | grep -oE -- '--needs [0-9a-f]+' | tail -1 | cut -d' ' -f2)"
"$stitch" app allow-threads Atelier --thread "$thread" --needs "$needs" | head -3
shown="$("$stitch" app allow-container Atelier --thread "$thread")"
echo "$shown" | head -40
build="$(echo "$shown" | grep -oE -- '--build [0-9a-f]+' | tail -1 | cut -d' ' -f2)"
run="$(echo "$shown" | grep -oE -- '--run [0-9a-f]+' | tail -1 | cut -d' ' -f2)"
"$stitch" app allow-container Atelier --thread "$thread" --build "$build" --run "$run"
"$stitch" app pin "$thread" >/dev/null
url="$("$stitch" app list | grep -oE 'http://[^ ]+' | head -1)"
host="$(echo "$url" | sed -E 's#http://([^/]+)/.*#\1#')"
echo "app: $host"

echo "waiting for the image…"
start=$(date +%s)
until "$stitch" app storage | grep -q "image(s)"; do
  sleep 5
  if [ $(( $(date +%s) - start )) -gt 1800 ]; then echo "no image after 30 minutes"; exit 1; fi
done
"$stitch" app storage
echo "built in $(( $(date +%s) - start ))s"

python3 "$here/drive.py" "$apps_port" "$host" "$(cat "$STITCH_DATA/apps.token")" \
  "$src/../../projects/technical-skirt/recipe.json" "$work"
echo "work: $work"
# KEEP=1: the node stays up for a look at the page (stop it with
# STITCH_DATA=$STITCH_DATA stitch node stop; its images with stitch app storage --rm Atelier).
if [ "${KEEP:-}" = 1 ]; then
  trap - EXIT
  echo "page: $url"
  echo "STITCH_DATA=$STITCH_DATA mock pid $mock"
fi
