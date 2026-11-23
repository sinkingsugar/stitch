#!/usr/bin/env bash
# Writes docs/cli.md from the CLI's own help, every command and subcommand.
# Usage: tools/cli-reference.sh [path/to/stitch]
set -euo pipefail

bin="${1:-target/debug/stitch}"
out="docs/cli.md"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

commands() {
  # The `Commands:` section of a help page, without `help`.
  "$bin" "$@" --help | awk '
    /^Commands:/ { on = 1; next }
    on && /^[^ ]/ { exit }
    on && /^  [a-z]/ { if ($1 != "help") print $1 }
  '
}

section() {
  local depth=$1
  shift
  local hashes
  hashes="$(printf '%*s' "$depth" '' | tr ' ' '#')"
  {
    printf '\n%s `stitch %s`\n\n```text\n' "$hashes" "$*"
    # The data directory's default depends on who generates it.
    "$bin" "$@" --help | sed -E 's/\[env: ([A-Z_]+)=[^]]*\]/[env: \1]/; s/[[:blank:]]+$//'
    printf '```\n'
  } >>"$tmp"
  local sub
  for sub in $(commands "$@"); do
    section $((depth + 1)) "$@" "$sub"
  done
}

{
  printf '# CLI reference\n\n'
  printf 'Generated from `stitch --help` by `tools/cli-reference.sh`; do not edit by hand.\n'
  printf 'Guides: [setup](setup.md) and [features](features.md).\n\n'
  printf '## `stitch`\n\n```text\n'
  "$bin" --help | sed -E 's/\[env: ([A-Z_]+)=[^]]*\]/[env: \1]/; s/[[:blank:]]+$//'
  printf '```\n'
} >"$tmp"
for c in $(commands); do
  section 3 "$c"
done
chmod 644 "$tmp"
mv "$tmp" "$out"
trap - EXIT
echo "wrote $out"
