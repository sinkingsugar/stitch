#!/usr/bin/env bash
# A scratch node for measuring sync: a copy of this machine's records
# (home, projects, folders; not the blobs, not the big shot folders) under a
# new key and name, on a fixed port, direct addresses only, no interval
# sync. It never reaches the user's real nodes: they do not know its key,
# and with `discovery = "local"` it has no addresses for them.
#
#   scratch.sh <dir> <name> <port> [src]
#   then: stitch --data <dir> peer add <other> <id> <ip:port>
#         stitch --data <dir> node
set -euo pipefail
dir=$1 name=$2 port=$3 src=${4:-$HOME/.stitch}
mkdir -p "$dir"
for d in home projects folders; do
  rsync -a --delete --exclude '*-shots' --exclude 'journal/' "$src/$d/" "$dir/$d/"
done
cat >"$dir/config.toml" <<EOF
node = "$name"
port = $port
discovery = "local"
blobs = "lazy"
sync_interval_secs = 0
EOF
stitch --data "$dir" id
