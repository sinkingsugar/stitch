#!/usr/bin/env bash
# Builds `stitch` and `stitch-shell-helper` (release, --locked) and puts them
# in ~/.cargo/bin (`just install`, and the self-update): staged beside them,
# signed there on macOS, then the helper and the binary each renamed into
# place, so a node watching its binary never sees half a build or one that
# is signed after the fact.
#
# On macOS `stitch` is signed with an Apple Development identity (or
# STITCH_SIGN_IDENTITY; `-` leaves it ad-hoc), so the keychain's "Always
# Allow" outlives rebuilds: an ad-hoc signature is the binary's hash, a new
# app to the keychain each time. Signing never asks: with no terminal, a
# keychain that would ask leaves it ad-hoc (said).
#
# Otter and engram are built as Cargo.lock pins them: refused while
# `just dev-link` points the build at checkouts beside this one (a build
# installed from those is nobody's commit).
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ -f .cargo/dev-link.toml ]; then
  echo "dev-link is active (.cargo/dev-link.toml): not installing; just dev-unlink first" >&2
  exit 1
fi
bindir="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin"
cargo build --release --locked -p stitch-cli
target="${CARGO_TARGET_DIR:-$PWD/target}/release"
mkdir -p "$bindir"
stage=$(mktemp -d "$bindir/.stitch-install.XXXXXX")
trap 'rm -rf "$stage"' EXIT
cp "$target/stitch" "$target/stitch-shell-helper" "$stage/"
if [ "$(uname)" = Darwin ]; then
  id="${STITCH_SIGN_IDENTITY:-$(security find-identity -v -p codesigning | awk '/"Apple Development/ { print $2; exit }')}"
  if [ -z "$id" ] || [ "$id" = - ]; then
    echo "stitch stays ad-hoc signed (no Apple Development identity: the keychain asks again after each rebuild)"
  elif codesign -f -s "$id" --identifier dev.stitch.cli "$stage/stitch" </dev/null; then
    echo "signed stitch ($id)"
  else
    echo "stitch stays ad-hoc signed: codesign failed (a keychain that asks, with nobody to answer)"
    cp "$target/stitch" "$stage/stitch"
  fi
fi
mv -f "$stage/stitch-shell-helper" "$bindir/stitch-shell-helper"
mv -f "$stage/stitch" "$bindir/stitch"
echo "installed $("$bindir/stitch" --version) in $bindir"
