#!/usr/bin/env bash
# Install with rustup only what is missing. `rustup target add` and
# `rustup toolchain install` write ~/.rustup/tmp even when all is installed,
# which a sandboxed shell refuses; listing only reads.
#
#   tools/rustup-ensure.sh targets <target>...              (the active toolchain)
#   tools/rustup-ensure.sh toolchain <name> [<component>...] (profile minimal)
set -euo pipefail

case "${1:-}" in
targets)
  shift
  installed=$(rustup target list --installed)
  missing=()
  for t in "$@"; do
    grep -qxF "$t" <<<"$installed" || missing+=("$t")
  done
  [ ${#missing[@]} -eq 0 ] || rustup target add "${missing[@]}"
  ;;
toolchain)
  tc=$2
  shift 2
  # Fails when the toolchain is not installed (never installing it).
  if have=$(RUSTUP_AUTO_INSTALL=0 rustup "+$tc" component list --installed 2>/dev/null); then
    ok=1
    for c in "$@"; do
      grep -qE "^$c(-|\$)" <<<"$have" || ok=0
    done
    [ $ok -eq 1 ] && exit 0
  fi
  args=()
  for c in "$@"; do args+=(--component "$c"); done
  rustup toolchain install "$tc" --profile minimal "${args[@]}"
  ;;
*)
  echo "usage: $0 targets <target>... | toolchain <name> [<component>...]" >&2
  exit 2
  ;;
esac
