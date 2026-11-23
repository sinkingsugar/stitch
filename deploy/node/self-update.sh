#!/usr/bin/env bash
# Follow Stitch on one branch of its origin (main, or the rollout's), and
# when the installed build is not its commit, build and install (--locked:
# Otter and engram as its Cargo.lock pins them, fetched by Cargo, no
# checkouts of theirs needed). The running node sees the new build and
# restarts into it once idle (under its service: exits for the service to
# start it again).
# Installed as ~/.local/bin/stitch-self-update by `stitch node service
# install --update`, run by stitch-update.timer (systemd) or the
# dev.stitch.update LaunchAgent (macOS); by hand: stitch-self-update
# [--force: build again a build that failed]
#
# Only from a clean checkout on main that holds exactly origin/<branch>,
# without `just dev-link`'s overrides: a checkout someone works in
# (changes, commits of its own, a diverged history, a link) is left alone,
# and it says why. "Changed" is the installed build (`stitch --version`)
# not being the checkout's label (tools/build-label.sh); a build that
# failed is not tried again until a commit moves (or --force).
#
# What lands on main was approved there (a fixer's change to a protected
# path asks the user before the node lands it, docs/self-improve.md §4):
# this script installs what origin/<branch> holds. A failed build, a
# failed promotion, TestFlight failing are told to the user once per
# commit (`stitch notify`).
#
# The canary finishes a build's trial (and promotes it) before taking a
# newer main, for at most STITCH_TRIAL_HOLD seconds (default 7200), so a
# main that moves every few minutes cannot keep `fleet` from moving.
#
# The rollout (docs/self-improve.md §4, 2), canary first: the canary follows
# main and promotes what ran healthy there for STITCH_PROMOTE_AFTER (the
# running process, continuously) to the next branch, the same commit (its
# Cargo.lock carries Otter's and engram's); the next nodes follow that
# branch, and may promote further:
#   tower:  STITCH_PROMOTE=fleet STITCH_APP=<app checkout>
#   mini:   STITCH_FOLLOW=fleet STITCH_PROMOTE=fleet-mac STITCH_APP=<app checkout>
#   laptop: STITCH_FOLLOW=fleet-mac STITCH_TESTFLIGHT=1 STITCH_APP=<app checkout>
#           (the phone after it)
# The app's checkout rides along on each (`service install --app`), so a
# commit landed in the app's repository alone is promoted, branch by
# branch, to the Mac that builds it (docs/self-improve.md §6).
#
# The app is another repository (its own checkout, STITCH_APP), built
# against this one. Where it is set, that checkout is held to the same
# rules (clean, on main, exactly origin/$STITCH_APP_FOLLOW, the same branch
# as this one's by default), fast-forwarded
# with this one and promoted with it; TestFlight runs its justfile by path
# (`just -f "$STITCH_APP/justfile"`), never whichever justfile is in the
# current directory, with STITCH_DIR set to this checkout.
#
# STITCH_CHECKOUT       the checkout (default ~/devel/stitch), on main
# STITCH_DATA           the node's data directory (default ~/.stitch)
# STITCH_FOLLOW         the branch of origin it follows (default main)
# STITCH_PROMOTE        a branch it pushes its build's commits to once healthy
# STITCH_PROMOTE_AFTER  seconds healthy before that (default 1800)
# STITCH_TRIAL_HOLD     seconds a build in its trial (healthy, not yet
#                       promoted) holds a newer main back (default 7200)
# STITCH_TESTFLIGHT     1: once the build is healthy here, the app
#                       checkout's `just ios device` and `just testflight`
#                       (the phone follows); at most 3 tries a commit pair
#                       (this checkout's and the app's), an hour apart, then
#                       the user is told
# STITCH_APP            the app checkout (`service install --app <checkout>`
#                       or `--testflight <checkout>`); unset with
#                       STITCH_TESTFLIGHT=1: this checkout's own justfile
#                       when it has the app (apps/ios), else nothing is
#                       uploaded
# STITCH_APP_FOLLOW     the branch of the app's origin it follows (default
#                       STITCH_FOLLOW's: the app is promoted with the core)
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
repos=(.)
follows=("$follow")
app=${STITCH_APP:-}
case $app in "~/"*) app="$HOME/${app#\~/}" ;; esac
if [ -n "$app" ]; then
  repos+=("$app")
  follows+=("${STITCH_APP_FOLLOW:-$follow}")
fi
echo "--- $(date '+%Y-%m-%d %H:%M:%S') following origin/$follow${app:+ (app: $app, origin/${STITCH_APP_FOLLOW:-$follow})}"

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

# The checkout, exactly origin/$follow, or why not. (Before Otter and
# engram were git dependencies it was three checkouts: an older script
# still checks ../otter and ../engram once, then runs this one.)
for i in "${!repos[@]}"; do
  repo=${repos[$i]}
  follow=${follows[$i]}
  [ -d "$repo/.git" ] || [ -f "$repo/.git" ] || skip "$repo: not a checkout: not updating"
  branch=$(git -C "$repo" rev-parse --abbrev-ref HEAD)
  [ "$branch" = main ] || skip "$repo is on $branch, not main: not updating"
  if [ -n "$(git -C "$repo" status --porcelain --untracked-files=no --ignore-submodules=all)" ]; then
    skip "$repo has changes: not updating"
  fi
  if [ -f "$repo/.cargo/dev-link.toml" ]; then
    skip "$repo: just dev-link is active (.cargo/dev-link.toml): not updating; just dev-unlink"
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
follow=${follows[0]}
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

# The label the build would say (crates/stitch-cli/build.rs, as
# tools/build-label.sh says it: Stitch's commit, Otter's and engram's pins
# in Cargo.lock), and what the installed binary says.
want="$(tools/build-label.sh .)"
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

# The trial of the installed build (written after each install): its label,
# the time it was installed, and each repo's HEAD commit (in `repos` order).
# A build's trial is finished, and the build promoted, before a newer main
# is taken: the next run is ~30 minutes after the restart, by when main has
# usually moved, and a trial that restarts with every commit never ends.
trial_file="$HOME/.cache/stitch-self-update.trial"
trial_is_installed() {
  [ -n "$have" ] && [ -f "$trial_file" ] && [ "$(sed -n 1p "$trial_file")" = "$have" ]
}

# What ran healthy here long enough goes on to the next nodes: this exact
# commit, built, installed, and running healthy for the whole trial (its
# Cargo.lock pins the Otter and engram it was built with). The commits are
# the trial's when the installed build is the one it recorded, else the
# checkouts' HEADs (the installed build must then be origin/$follow's).
promote() {
  [ -n "${STITCH_PROMOTE:-}" ] || return 0
  local from_trial=0
  if trial_is_installed; then
    from_trial=1
  elif [ "$have" != "$want" ]; then
    echo "not promoting: the installed build is not origin/$follow's"; return 0
  fi
  "$bin" --data "$data" node builds --healthy-for "${STITCH_PROMOTE_AFTER:-1800}" >/dev/null 2>&1 \
    || { echo "not promoting yet: not healthy here for long enough"; return 0; }
  local repo head remote out i
  local heads=()
  for i in "${!repos[@]}"; do
    head=
    [ "$from_trial" = 1 ] && head=$(sed -n "$((i + 3))p" "$trial_file")
    # (a repo the trial did not record, STITCH_APP set since: HEAD, when
    # the installed build is origin/$follow's)
    [ -n "$head" ] || [ "$have" != "$want" ] || head=$(git -C "${repos[$i]}" rev-parse HEAD)
    [ -n "$head" ] || { echo "not promoting: the trial file has no commit for ${repos[$i]}"; return 0; }
    heads+=("$head")
  done
  for i in "${!repos[@]}"; do
    repo=${repos[$i]}
    head=${heads[$i]}
    git -C "$repo" fetch --quiet origin "$STITCH_PROMOTE" 2>/dev/null || true
    remote=$(git -C "$repo" rev-parse --verify --quiet "origin/$STITCH_PROMOTE" || true)
    [ "$remote" = "$head" ] && continue
    if [ -n "$remote" ] && ! git -C "$repo" merge-base --is-ancestor "$remote" "$head"; then
      tell promote "not promoting: $repo's origin/$STITCH_PROMOTE is not behind $head"
      return 0
    fi
  done
  for i in "${!repos[@]}"; do
    repo=${repos[$i]}
    head=${heads[$i]}
    if out=$(git -C "$repo" push --quiet origin "$head:refs/heads/$STITCH_PROMOTE" 2>&1); then
      echo "$repo: promoted $(git -C "$repo" log --oneline -1 "$head") to $STITCH_PROMOTE"
    else
      tell promote "promoting $repo to $STITCH_PROMOTE failed: $out"
      return 0
    fi
  done
}

# The installed build is in its trial: it is the recorded one, not yet
# promoted, healthy now, not past the trial, and installed less than
# STITCH_TRIAL_HOLD ago (so a build that keeps restarting cannot hold main
# back forever).
in_trial() {
  trial_is_installed || return 1
  local i repo head remote now at promoted=1
  for i in "${!repos[@]}"; do
    repo=${repos[$i]}
    head=$(sed -n "$((i + 3))p" "$trial_file")
    remote=$(git -C "$repo" rev-parse --verify --quiet "origin/$STITCH_PROMOTE" || true)
    if [ -z "$remote" ] || ! git -C "$repo" merge-base --is-ancestor "$head" "$remote" 2>/dev/null; then
      promoted=0
    fi
  done
  [ "$promoted" = 0 ] || return 1  # already promoted (or passed)
  "$bin" --data "$data" node builds --healthy-for 0 >/dev/null 2>&1 || return 1
  ! "$bin" --data "$data" node builds --healthy-for "${STITCH_PROMOTE_AFTER:-1800}" >/dev/null 2>&1 || return 1
  now=$(date +%s)
  at=$(sed -n 2p "$trial_file")
  [ $((now - ${at:-0})) -lt "${STITCH_TRIAL_HOLD:-7200}" ]
}

# The phone: once this commit's build is healthy here, uploaded once per
# pair of commits (this checkout's, the app's); at most 3 tries a pair, an
# hour apart at least, then the user is told. The app's justfile by path,
# with STITCH_DIR this checkout: its `testflight` checks the app's core pin
# and records app SHA, core SHA and the build number together.
testflight() {
  [ "${STITCH_TESTFLIGHT:-}" = 1 ] || return 0
  local stamp="$data/testflight-commit" tries_file="$data/testflight-tries" head tries last now justfile
  if [ -n "$app" ]; then
    justfile="$app/justfile"
    head="$(git rev-parse HEAD)+$(git -C "$app" rev-parse HEAD)"
  elif [ -d apps/ios ]; then
    # A checkout that holds the app itself.
    justfile="$PWD/justfile"
    head=$(git rev-parse HEAD)
  else
    tell testflight "TestFlight: no app checkout (STITCH_APP): nothing uploaded; \`stitch node service install --update … --testflight <app checkout>\`"
    return 0
  fi
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
  echo "testflight: $justfile (core $(git rev-parse --short=12 HEAD)${app:+, app $(git -C "$app" rev-parse --short=12 HEAD)})"
  if STITCH_DIR="$PWD" just -f "$justfile" ios device && STITCH_DIR="$PWD" just -f "$justfile" testflight; then
    echo "$head" >"$stamp"
  elif [ "$tries" -ge 3 ]; then
    tell testflight "TestFlight failed 3 times for $(git log --oneline -1 HEAD | cut -c1-60)${app:+ (app $(git -C "$app" log --oneline -1 HEAD | cut -c1-40))}: not tried again for this commit"
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
if [ -n "${STITCH_PROMOTE:-}" ] && [ "${1:-}" != "--force" ] && trial_is_installed; then
  promote
  if in_trial; then
    echo "holding $want: $have is in its trial (not promoted yet)"
    exit 0
  fi
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
# Its trial begins: the commits it was built from (promoted by a later run,
# before a newer main is taken).
{
  echo "$have"
  date +%s
  for repo in "${repos[@]}"; do git -C "$repo" rev-parse HEAD; done
} >"$trial_file"
# The node restarts into it once idle (it looks every 10 s, and wants the
# binary unchanged over two looks); under a service, the service starts it
# again. `jobs` starts the node (or the service) if none runs.
sleep "${STITCH_UPDATE_SETTLE:-25}"
stitch --data "$data" jobs >/dev/null || true
stitch --data "$data" node status | sed -n '1,4p' || true
# The phone: at the next runs, once this build is healthy here.
