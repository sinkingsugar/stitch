#!/usr/bin/env bash
# The build's label (`STITCH_BUILD`, docs/self-improve.md §2.1), as
# crates/stitch-cli/build.rs makes it: Stitch's commit (`-dirty` when
# tracked files changed), then Otter's and engram's as Cargo.lock pins them,
#   <stitch> otter <rev> engram <rev>
# or, while `just dev-link` points one at a checkout beside this one, that
# checkout's commit and "(linked)":
#   <stitch> otter <commit>[-dirty] (linked) engram <rev>
# All commits 12 hex digits. For the shell's builds (the app's build of `stitch-ffi`)
# and the self-update's "is the installed build this checkout's"; a test in
# stitch-cli (tests/self_update.rs) holds it to build.rs.
#   build-label.sh [checkout]   (default: the one this script is in)
set -euo pipefail
cd "${1:-$(dirname "$0")/..}"

commit() {
  local c
  c=$(git -C "$1" rev-parse --short=12 HEAD 2>/dev/null) || return 0
  [ -n "$(git -C "$1" status --porcelain --untracked-files=no 2>/dev/null)" ] && c="$c-dirty"
  printf '%s' "$c"
}

label=$(commit .)
label=${label:-unknown}
for dep in otter engram; do
  link=$(sed -n "s/^# link $dep //p" .cargo/dev-link.toml 2>/dev/null || true)
  if [ -n "$link" ]; then
    c=$(commit "$link")
    label="$label $dep ${c:-unknown} (linked)"
    continue
  fi
  rev=$(sed -nE "s|^source = \"git\+https://github\.com/sinkingsugar/$dep(\?[^#]*)?#([0-9a-f]{12})[0-9a-f]*\"$|\2|p" Cargo.lock 2>/dev/null | head -n1)
  [ -n "$rev" ] && label="$label $dep $rev"
done
printf '%s\n' "$label"
