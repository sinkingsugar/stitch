#!/usr/bin/env bash
# The fast checks while developing (`just quick`), for what changed against
# a base (default main): the commits since the branch left it, uncommitted
# and untracked files.
#   quick.sh [base]
#
# - formatting (cargo fmt --check), always;
# - clippy (-D warnings, all targets) on the changed crates;
# - their tests;
# - apps/client changed: the web client rebuilt (`just client`: needs bun, ~/.bun/bin)
#   gives the dist/ files it has (typecheck included); untouched, no bun is needed.
# Cargo.toml, Cargo.lock or rust-toolchain.toml changed: the whole workspace.
# `just check` is the full set. Logs: target dir's quick/.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
source tools/steps.sh
base=${1:-main}
# Otter or engram from a checkout beside this one (`just dev-link`): said
# loudly, at the start and again at the end.
tools/dev-link.sh warn

changed=$( {
  git diff --name-only "$base...HEAD"
  git diff --name-only HEAD
  git ls-files --others --exclude-standard
} | sort -u)

crates=()
workspace=false
client=false
while IFS= read -r f; do
  case $f in
    crates/*/*)
      c=${f#crates/}; c=${c%%/*}
      [[ " ${crates[*]-} " == *" $c "* ]] || crates+=("$c") ;;
    Cargo.toml | Cargo.lock | rust-toolchain.toml | .cargo/*) workspace=true ;;
    apps/client/*) client=true ;;
  esac
done <<<"$changed"

if [ -z "$changed" ]; then
  echo "nothing changed against $base"
  exit 0
fi
echo "changed against $base: $(echo "$changed" | wc -l | tr -d ' ') files" \
  "(crates: ${crates[*]:-none}; workspace files: $workspace; web client: $client)"

target=${CARGO_TARGET_DIR:-$root/target}
steps_init "$target/quick/$(basename "$root")"

pkgs=()
if $workspace; then
  pkgs=(--workspace)
else
  for c in "${crates[@]+"${crates[@]}"}"; do
    # A crate that is gone (deleted or renamed) is not built.
    [ -f "crates/$c/Cargo.toml" ] && pkgs+=(-p "$c")
  done
fi

rust() {
  step fmt cargo fmt -- --check || return
  if [ ${#pkgs[@]} -eq 0 ]; then
    step_skip clippy "no crate changed"
    step_skip test "no crate changed"
    return
  fi
  step clippy cargo clippy "${pkgs[@]}" --all-targets -- -D warnings || return
  # stitch-node's tests find stitch-cli's shell helper beside cargo's deps/;
  # `cargo test -p` does not build another crate's binary (only --workspace does).
  step helper cargo build -p stitch-cli --bin stitch-shell-helper || return
  step test cargo test "${pkgs[@]}"
}

web() {
  # dist/ and dist/dashboard/ (the built-in dashboard's entry).
  dist_sum() { find apps/client/dist -type f | sort | xargs cat apps/client/src/version.ts; }
  local before; before=$(dist_sum | cksum)
  step client just client || return
  [ "$(dist_sum | cksum)" = "$before" ] || step client-stale bash -c 'echo "dist/ differed from a rebuild: it is rebuilt now; commit it"; exit 1'
}

chain rust rust
if $client; then chain web web; fi
steps_wait
tools/dev-link.sh warn
steps_summary
