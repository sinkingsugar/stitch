#!/usr/bin/env bash
# Follow Stitch, Otter and engram on one branch of their origins (main, or
# the rollout's), and when the installed build is not theirs, build and
# install. The running node sees the new build and restarts into it once
# idle (under its service: exits for the service to start it again).
# Installed as ~/.local/bin/stitch-self-update by `stitch node service
# install --update`, run by stitch-update.timer (systemd) or the
# dev.stitch.update LaunchAgent (macOS); by hand: stitch-self-update
# [--force: build again a build that failed]
#
# Only from clean checkouts on main that hold exactly origin/<branch>: a
# checkout someone works in (changes, commits of its own, a diverged
# history) is left alone, and it says why. "Changed" is the installed build
# (`stitch --version`) not being the checkouts' HEADs; a build that failed
# is not tried again until a commit moves (or --force).
#
# What lands on main was approved there (a fixer's change to a protected
# path asks the user before the node lands it, docs/self-improve.md §4):
# this script installs what origin/<branch> holds. A failed build, a
# failed promotion, TestFlight failing are told to the user once per
# commit (`stitch notify`).
#
# The rollout (docs/self-improve.md §4, 2), canary first: the canary follows
# main and promotes what ran healthy there for STITCH_PROMOTE_AFTER (the
# running process, continuously) to the next branch, the same commits of
# all three; the next nodes follow that branch, and may promote further:
#   gamer: STITCH_PROMOTE=fleet
#   geek:  STITCH_FOLLOW=fleet STITCH_PROMOTE=fleet-mac
#   mbp:   STITCH_FOLLOW=fleet-mac STITCH_TESTFLIGHT=1   (the phone after it)
#
# STITCH_CHECKOUT       the checkout (default ~/devel/stitch), on main
# STITCH_DATA           the node's data directory (default ~/.stitch)
# STITCH_FOLLOW         the branch of origin it follows (default main)
# STITCH_PROMOTE        a branch it pushes its build's commits to once healthy
# STITCH_PROMOTE_AFTER  seconds healthy before that (default 1800)
# STITCH_TESTFLIGHT     1: once the build is healthy here, `just ios device`
#                       and `just testflight` (the phone follows); at most
#                       3 tries a commit, an hour apart, then the user is
#                       told
# STITCH_UPDATE_SETTLE  seconds to wait for the node to restart (default 25)
set -euo pipefail
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"
export GIT_TERMINAL_PROMPT=0
mkdir -p "$HOME/.cache"
exec 9>"$HOME/.cache/stitch-self-update.lock"
if command -v flock >/dev/null; then
  flock -n 9 || { echo "another update runs"; exit 0; }
fi
cd "${STITCH_CHECKOUT:-$HOME/devel/stitch}"
data=${STITCH_DATA:-$HOME/.stitch}
follow=${STITCH_FOLLOW:-main}
bin="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin/stitch"
failed_mark="$HOME/.cache/stitch-self-update.failed"
repos=(. ../otter ../engram)
echo "--- $(date '+%Y-%m-%d %H:%M:%S') following origin/$follow"

# Not updating, and why. A checkout worked in pauses the rollout (and
# TestFlight after it) without a sound: when that has lasted
# STITCH_STUCK_AFTER seconds (default 3 hours) while origin/$follow has a
# Stitch commit the installed build is not, the user is told, once until a
# run gets past the checks again.
skipping="$HOME/.cache/stitch-self-update.skipping"
skip() {
  echo "$1"
  local now since remote installed
  now=$(date +%s)
  [ -s "$skipping" ] || echo "$now" >"$skipping"
  since=$(head -n1 "$skipping")
  if [ $((now - since)) -ge "${STITCH_STUCK_AFTER:-10800}" ] && ! grep -qx told "$skipping"; then
    git fetch --quiet origin "$follow" 2>/dev/null || true
    remote=$(git rev-parse --short=12 "origin/$follow" 2>/dev/null || true)
    installed=$("$bin" --version 2>/dev/null | sed -n 's/^stitch [^ ]* (\([^ ]*\) .*$/\1/p' || true)
    if [ -n "$remote" ] && [ "$remote" != "$installed" ]; then
      echo told >>"$skipping"
      local what="updates"
      [ "${STITCH_TESTFLIGHT:-}" = 1 ] && what="updates and TestFlight uploads"
      echo "stuck: told the user"
      "$bin" --data "$data" notify --title "Stitch update on $(hostname -s 2>/dev/null || hostname) stuck" \
        "No $what for $(((now - since) / 3600))h: $1. origin/$follow has $remote, installed ${installed:-none}." \
        >/dev/null 2>&1 || true
    fi
  fi
  exit 0
}

# The checkouts, each exactly origin/$follow, or why not.
for repo in "${repos[@]}"; do
  [ -d "$repo/.git" ] || [ -f "$repo/.git" ] || skip "$repo: not a checkout: not updating"
  branch=$(git -C "$repo" rev-parse --abbrev-ref HEAD)
  [ "$branch" = main ] || skip "$repo is on $branch, not main: not updating"
  if [ -n "$(git -C "$repo" status --porcelain --untracked-files=no --ignore-submodules=all)" ]; then
    skip "$repo has changes: not updating"
  fi
  if ! git -C "$repo" fetch --quiet origin "$follow"; then
    skip "$repo: no origin/$follow yet: nothing to follow"
  fi
  remote=$(git -C "$repo" rev-parse "origin/$follow")
  if ! git -C "$repo" merge --ff-only --quiet "origin/$follow" 2>/dev/null; then
    skip "$repo: its main and origin/$follow have diverged ($(git -C "$repo" rev-parse --short HEAD) vs $(git -C "$repo" rev-parse --short "$remote")): not updating; put main back on origin/$follow"
  fi
  if [ "$(git -C "$repo" rev-parse HEAD)" != "$remote" ]; then
    skip "$repo: its main has commits origin/$follow has not ($(git -C "$repo" log --oneline "$remote..HEAD" | wc -l | tr -d ' ')): not updating; only what is on origin is built"
  fi
done
rm -f "$skipping"

# This script, as origin/$follow has it, for the next run (written only by
# `service install` otherwise): a new file moved over it, so this run's bash
# keeps reading the old one.
installed_script="$HOME/.local/bin/stitch-self-update"
if [ -f deploy/node/self-update.sh ] && [ -f "$installed_script" ] \
  && ! cmp -s deploy/node/self-update.sh "$installed_script"; then
  cp deploy/node/self-update.sh "$installed_script.new" && chmod 755 "$installed_script.new" \
    && mv -f "$installed_script.new" "$installed_script" && echo "self-update script refreshed"
fi

# The label the build would say (crates/stitch-cli/build.rs), and what the
# installed binary says.
want="$(git rev-parse --short=12 HEAD) otter $(git -C ../otter rev-parse --short=12 HEAD) engram $(git -C ../engram rev-parse --short=12 HEAD)"
have=$("$bin" --version 2>/dev/null | sed -n 's/^stitch [^ ]* (\(.*\))$/\1/p' || true)
notified="$HOME/.cache/stitch-self-update.notified"

# Tells the user once per commit and reason (`stitch notify`).
tell() {
  local key="$1 $want" body=$2
  echo "$body"
  grep -qxF "$key" "$notified" 2>/dev/null && return 0
  echo "$key" >>"$notified"
  "$bin" --data "$data" notify --title "Stitch update on $(hostname -s 2>/dev/null || hostname)" "$body" >/dev/null 2>&1 || true
}

# What ran healthy here long enough goes on to the next nodes: these exact
# commits, built, installed, and running healthy for the whole trial.
# Otter and engram first: a node following the branch never sees Stitch's
# commit before the others it was built with.
promote() {
  [ -n "${STITCH_PROMOTE:-}" ] || return 0
  [ "$have" = "$want" ] || { echo "not promoting: the installed build is not origin/$follow's"; return 0; }
  "$bin" --data "$data" node builds --healthy-for "${STITCH_PROMOTE_AFTER:-1800}" >/dev/null 2>&1 \
    || { echo "not promoting yet: not healthy here for long enough"; return 0; }
  local repo head remote out
  local order=(../otter ../engram .)
  for repo in "${order[@]}"; do
    head=$(git -C "$repo" rev-parse HEAD)
    git -C "$repo" fetch --quiet origin "$STITCH_PROMOTE" 2>/dev/null || true
    remote=$(git -C "$repo" rev-parse --verify --quiet "origin/$STITCH_PROMOTE" || true)
    [ "$remote" = "$head" ] && continue
    if [ -n "$remote" ] && ! git -C "$repo" merge-base --is-ancestor "$remote" "$head"; then
      tell promote "not promoting: $repo's origin/$STITCH_PROMOTE is not behind $head"
      return 0
    fi
  done
  for repo in "${order[@]}"; do
    head=$(git -C "$repo" rev-parse HEAD)
    if out=$(git -C "$repo" push --quiet origin "$head:refs/heads/$STITCH_PROMOTE" 2>&1); then
      echo "$repo: promoted $(git -C "$repo" log --oneline -1 "$head") to $STITCH_PROMOTE"
    else
      tell promote "promoting $repo to $STITCH_PROMOTE failed: $out"
      return 0
    fi
  done
}

# The phone: once this commit's build is healthy here, uploaded once; at
# most 3 tries a commit, an hour apart at least, then the user is told.
testflight() {
  [ "${STITCH_TESTFLIGHT:-}" = 1 ] || return 0
  local stamp="$data/testflight-commit" tries_file="$data/testflight-tries" head tries last now
  head=$(git rev-parse HEAD)
  [ "$(cat "$stamp" 2>/dev/null)" = "$head" ] && return 0
  [ "$have" = "$want" ] || return 0
  read -r tc tries last 2>/dev/null <"$tries_file" || true
  [ "${tc:-}" = "$head" ] || { tries=0; last=0; }
  now=$(date +%s)
  if [ "${tries:-0}" -ge 3 ]; then
    return 0
  fi
  if [ $((now - ${last:-0})) -lt "${STITCH_TESTFLIGHT_BACKOFF:-3600}" ]; then
    echo "testflight: tried less than an hour ago"
    return 0
  fi
  "$bin" --data "$data" node builds --healthy-for 0 >/dev/null 2>&1 \
    || { echo "testflight: not yet, the build is not healthy here"; return 0; }
  mkdir -p "$data"
  tries=$(( ${tries:-0} + 1 ))
  echo "$head $tries $now" >"$tries_file"
  if just ios device && just testflight; then
    echo "$head" >"$stamp"
  elif [ "$tries" -ge 3 ]; then
    tell testflight "TestFlight failed 3 times for $(git log --oneline -1 HEAD | cut -c1-60): not tried again for this commit"
  else
    echo "testflight failed ($tries of 3): tried again in an hour"
  fi
}

if [ "$have" = "$want" ] && [ "${1:-}" != "--force" ]; then
  echo "up to date ($want)"
  promote
  testflight
  exit 0
fi
echo "installed: ${have:-none}; wanted: $want"
if [ -z "${1:-}" ] && [ -f "$failed_mark" ] && [ "$(head -1 "$failed_mark")" = "$want" ]; then
  echo "not tried again until a commit moves (--force): $(sed -n 2p "$failed_mark")"
  exit 0
fi

git submodule update --init --quiet
# Built first, put in place only once it built: a failed build leaves the
# running node and its binary as they are.
if ! deploy/node/install.sh; then
  mkdir -p "$(dirname "$failed_mark")"
  printf '%s\n%s\n' "$want" "its build failed" >"$failed_mark"
  tell build "the build of $(git log --oneline -1 HEAD | cut -c1-60) failed: not tried again until a commit moves (stitch-self-update --force)"
  exit 1
fi
rm -f "$failed_mark"
have=$("$bin" --version 2>/dev/null | sed -n 's/^stitch [^ ]* (\(.*\))$/\1/p' || true)
# The node restarts into it once idle (it looks every 10 s, and wants the
# binary unchanged over two looks); under a service, the service starts it
# again. `jobs` starts the node (or the service) if none runs.
sleep "${STITCH_UPDATE_SETTLE:-25}"
stitch --data "$data" jobs >/dev/null || true
stitch --data "$data" node status | sed -n '1,4p' || true
# The phone: at the next runs, once this build is healthy here.
