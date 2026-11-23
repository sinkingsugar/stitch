#!/usr/bin/env bash
# Times `stitch sync <peer>` on a scratch node (scratch.sh): nothing new,
# then one new record, N times each; prints seconds per run.
#   bench.sh <dir> <peer> [runs] [stitch binary]
set -euo pipefail
dir=$1 peer=$2 runs=${3:-5} bin=${4:-stitch}
t() { local s e; s=$(date +%s%N); "$@" >/dev/null; e=$(date +%s%N); echo $(((e - s) / 1000000)); }
echo "nothing new (ms):" $(for _ in $(seq "$runs"); do t "$bin" --data "$dir" sync "$peer"; done)
