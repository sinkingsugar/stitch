#!/usr/bin/env bash
# The Studio demo (examples/app-threads) end to end against a node of its
# own: a temporary data directory, apps on port 7277, a mock model
# (mock_model.py, here) on 7278. Nothing of the user's node is touched.
# Leaves the node running and prints the app's address;
# `STITCH_DATA=<dir> stitch node stop` ends it.
#   tools/app-threads/try.sh [path to the stitch binary]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
app="$repo/examples/app-threads"
stitch="${1:-$repo/target/debug/stitch}"
export STITCH_DATA="${STITCH_DATA_DEMO:-$(mktemp -d)/data}"
mock_port=7278
apps_port=7277

mkdir -p "$STITCH_DATA"
python3 "$here/mock_model.py" "$mock_port" "$app" >"$STITCH_DATA/../mock.log" 2>&1 &
mock=$!
trap 'kill $mock 2>/dev/null || true' EXIT

"$stitch" setup --name demo --local >/dev/null
cat >>"$STITCH_DATA/config.toml" <<EOF

sync_interval_secs = 0

[apps]
port = $apps_port

[providers.anthropic]
kind = "anthropic"
base_url = "http://127.0.0.1:$mock_port"
EOF
echo sk-mock | "$stitch" key set anthropic --file >/dev/null

out="$("$stitch" chat "Show the app in $app as Studio" 2>&1)"
echo "$out"
thread="$("$stitch" threads 2>/dev/null | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{6}Z-demo-[a-z0-9]+' | head -1)"
echo "conversation: $thread"
# Shown first (nothing written), then granted with the hash shown.
shown="$("$stitch" app allow-threads Studio --thread "$thread")"
echo "$shown"
needs="$(echo "$shown" | grep -oE -- '--needs [0-9a-f]+' | tail -1 | cut -d' ' -f2)"
"$stitch" app allow-threads Studio --thread "$thread" --needs "$needs"
"$stitch" app pin "$thread" --icon tshirt >/dev/null
"$stitch" app list
echo
echo "STITCH_DATA=$STITCH_DATA (stop it with: STITCH_DATA=$STITCH_DATA $stitch node stop)"
trap - EXIT
echo "mock model: pid $mock (kill it when done)"
