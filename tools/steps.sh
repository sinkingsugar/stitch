# Steps with logs and timings, for quick.sh (sourced; the app's scripts
# source it from the core checkout too).
#
#   steps_init <dir>          logs go to <dir>/<step>.log
#   step <name> <cmd…>        runs it, its output in its log
#   chain <name> <fn>         runs a shell function in the background: its
#                             steps in order, the chain stopping at a failure
#   steps_wait                waits for the chains
#   steps_summary             each step's time and result; fails if one did
#
# Cargo invocations share one target directory and wait on its lock, so
# one chain runs cargo; others run what does not touch it (xcodebuild).

STEPS_DIR=
STEPS_PIDS=()
STEPS_T0=$(date +%s)

steps_init() {
  STEPS_DIR=$1
  mkdir -p "$STEPS_DIR"
  : >"$STEPS_DIR/.order"
}

step() {
  local name=$1; shift
  local log="$STEPS_DIR/$name.log" t0 rc=0
  echo "$name" >>"$STEPS_DIR/.order"
  t0=$(date +%s)
  printf '\033[2m%s: started\033[0m\n' "$name" >&2
  { echo "\$ $*"; "$@"; } >"$log" 2>&1 </dev/null || rc=$?
  echo "$rc $(( $(date +%s) - t0 ))" >"$STEPS_DIR/$name.status"
  if [ "$rc" -eq 0 ]; then
    printf '%s: \033[32mok\033[0m (%ss)\n' "$name" "$(( $(date +%s) - t0 ))" >&2
  else
    printf '%s: \033[31mfailed\033[0m (%ss, %s)\n' "$name" "$(( $(date +%s) - t0 ))" "$log" >&2
  fi
  return "$rc"
}

# A step that did not run, and why.
step_skip() {
  echo "$1" >>"$STEPS_DIR/.order"
  echo "skip $2" >"$STEPS_DIR/$1.status"
  printf '%s: skipped (%s)\n' "$1" "$2" >&2
}

chain() {
  local fn=$2
  ( set +e; "$fn" ) &
  STEPS_PIDS+=($!)
}

steps_wait() {
  local p
  for p in "${STEPS_PIDS[@]+"${STEPS_PIDS[@]}"}"; do wait "$p" || true; done
  STEPS_PIDS=()
}

steps_summary() {
  local name rc secs failed=0 status
  echo
  printf '%-22s %6s  %s\n' step time result
  while read -r name; do
    [ -f "$STEPS_DIR/$name.status" ] || { printf '%-22s %6s  %s\n' "$name" - "did not finish"; failed=1; continue; }
    status=$(cat "$STEPS_DIR/$name.status")
    rc=${status%% *}
    secs=${status#* }
    if [ "$rc" = skip ]; then
      printf '%-22s %6s  skipped: %s\n' "$name" - "$secs"
    elif [ "$rc" -eq 0 ]; then
      printf '%-22s %5ss  \033[32mok\033[0m\n' "$name" "$secs"
    else
      printf '%-22s %5ss  \033[31mfailed\033[0m  %s\n' "$name" "$secs" "$STEPS_DIR/$name.log"
      failed=1
    fi
  done <"$STEPS_DIR/.order"
  printf 'total %ss; logs in %s\n' "$(( $(date +%s) - STEPS_T0 ))" "$STEPS_DIR"
  if [ "$failed" -ne 0 ]; then
    while read -r name; do
      status=$(cat "$STEPS_DIR/$name.status" 2>/dev/null || echo "1 0")
      case ${status%% *} in 0 | skip) continue ;; esac
      printf '\n\033[31m--- %s\033[0m (last lines)\n' "$name"
      grep -E "^(error|warning)|error:|FAILED|panicked|failed" "$STEPS_DIR/$name.log" | head -20
      tail -n 15 "$STEPS_DIR/$name.log"
    done <"$STEPS_DIR/.order"
    return 1
  fi
}
