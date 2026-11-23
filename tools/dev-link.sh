#!/usr/bin/env bash
# Cross-repo development: Otter and engram are git dependencies pinned by
# Cargo.lock; `just dev-link` points this checkout's build at the
# checkouts beside it instead (../otter, ../engram), `just dev-unlink` puts
# the pins back. Opt-in and never committed.
#
#   dev-link.sh link [otter] [engram]   (default: both, those that exist)
#   dev-link.sh unlink
#   dev-link.sh bump otter|engram       the pin to the newest commit of its main
#   dev-link.sh status                  says whether a link is active (exit 0 when it is)
#   dev-link.sh warn                    a loud warning on stderr while it is
#
# How: an untracked, gitignored .cargo/dev-link.toml with Cargo `[patch]`
# sections, which the tracked .cargo/config.toml includes when it exists
# (an optional config include): every cargo command in this checkout sees
# it (rust-analyzer too), nothing else does, and no command needs a flag.
# A `[patch]` drops the pinned `source` lines from Cargo.lock while it is
# active: the file remembers the pins (`# pin <dep> <rev>`), and unlink
# puts Cargo.lock back on them (`cargo update --precise`). A Cargo.lock
# committed while linked fails every `--locked` build (CI, self-update):
# unlink before committing.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
file=.cargo/dev-link.toml
url() { echo "https://github.com/sinkingsugar/$1"; }

# Beside the main checkout (a worktree's siblings are the same).
siblings() {
  local main
  main="$(git rev-parse --path-format=absolute --git-common-dir)/.."
  dirname "$(cd "$main" && pwd)"
}

# The packages Cargo.lock has from a dependency's git source.
packages() {
  awk -v src="source = \"git+$(url "$1")" '
    /^name = / { name = $3; gsub(/"/, "", name) }
    /^source = / && index($0, src) == 1 { c = substr($0, length(src) + 1, 1); if (c == "#" || c == "?") print name }
  ' Cargo.lock | sort -u
}

pin() {
  sed -nE "s|^source = \"git\+https://github\.com/sinkingsugar/$1(\?[^#]*)?#([0-9a-f]{40})\"$|\2|p" Cargo.lock | head -n1
}

case ${1:-status} in
  link)
    shift
    [ -f "$file" ] && { echo "already linked ($file): just dev-unlink first" >&2; exit 1; }
    deps=("$@")
    [ ${#deps[@]} -gt 0 ] || deps=(otter engram)
    dir=$(siblings)
    {
      echo "# Written by \`just dev-link\` (tools/dev-link.sh); \`just dev-unlink\` removes it."
      echo "# Untracked: Cargo.lock loses its pins while it is here."
    } >"$file.new"
    linked=()
    for dep in "${deps[@]}"; do
      case $dep in otter | engram) ;; *) echo "unknown dependency $dep (otter, engram)" >&2; rm -f "$file.new"; exit 1 ;; esac
      src="$dir/$dep"
      if [ ! -d "$src" ]; then
        echo "$dep: no checkout at $src: not linked" >&2
        continue
      fi
      rev=$(pin "$dep")
      [ -n "$rev" ] || { echo "$dep: Cargo.lock has no pin for $(url "$dep"): not linked" >&2; continue; }
      pkgs=$(packages "$dep" | tr '\n' ' ')
      printf '# link %s %s\n# pin %s %s %s\n[patch."%s"]\n' "$dep" "$src" "$dep" "$rev" "$pkgs" "$(url "$dep")" >>"$file.new"
      for p in $pkgs; do
        # The package's directory in the checkout: the Cargo.toml whose
        # [package] is named so. A `[patch]` is keyed by the package's name,
        # which a renamed package (engram's `engram-gate`, whose lib is still
        # `engram`) no longer shares with its lib: the `[lib]`'s `name` must
        # not match.
        m=$(cd "$src" && git ls-files '*Cargo.toml' | while read -r f; do
          awk -v p="$p" '
            /^\[/ { sec = $0 }
            sec == "[package]" && $0 ~ "^name *= *\"" p "\"" { found = 1; exit }
            END { exit !found }
          ' "$f" && { echo "$f"; break; }
        done || true)
        if [ -z "$m" ]; then
          echo "$dep: no package $p in $src (Cargo.lock pins it at ${rev:0:12}: is $src at a commit that renamed it? \`just bump-$dep\` first, or check that commit out there)" >&2
          rm -f "$file.new"
          exit 1
        fi
        printf '%s = { path = "%s" }\n' "$p" "$(dirname "$src/$m")" >>"$file.new"
      done
      linked+=("$dep")
    done
    [ ${#linked[@]} -gt 0 ] || { rm -f "$file.new"; echo "nothing linked" >&2; exit 1; }
    mv "$file.new" "$file"
    cargo metadata --format-version 1 >/dev/null
    echo "linked ${linked[*]} ($file); \`just dev-unlink\` before committing Cargo.lock"
    ;;
  unlink)
    [ -f "$file" ] || { echo "not linked"; exit 0; }
    pins=$(sed -n 's/^# pin //p' "$file")
    rm -f "$file"
    # Linked, Cargo.lock has the linked packages without a `source` line:
    # each gets its pin's back, which is Cargo.lock exactly as it was (a
    # resolution would also move unrelated packages within their ranges).
    while read -r dep rev pkgs; do
      [ -n "$dep" ] || continue
      awk -v src="source = \"git+$(url "$dep")#$rev\"" -v pkgs=" $pkgs " '
        /^\[\[package\]\]$/ { inpkg = 1; name = ""; has = 0 }
        inpkg && /^name = / { name = $3; gsub(/"/, "", name) }
        inpkg && /^source = / { has = 1 }
        { print }
        inpkg && /^version = / && !has && index(pkgs, " " name " ") { print src; has = 1 }
        /^$/ { inpkg = 0 }
      ' Cargo.lock >Cargo.lock.new && mv Cargo.lock.new Cargo.lock
      echo "$dep: back on $(url "$dep") at ${rev:0:12}"
    done <<<"$pins"
    # What changed while linked (a dependency the checkout added) the next
    # resolution settles; said, since it changes Cargo.lock.
    if ! cargo metadata --format-version 1 --locked >/dev/null 2>&1; then
      cargo metadata --format-version 1 >/dev/null
      echo "Cargo.lock changed while linked: resolved again; review \`git diff Cargo.lock\`" >&2
    fi
    ;;
  bump)
    # `just bump-otter` / `just bump-engram`: the pin moved to the newest
    # commit of the dependency's default branch (main), every package of it.
    dep=${2:?bump otter|engram}
    case $dep in otter | engram) ;; *) echo "unknown dependency $dep (otter, engram)" >&2; exit 1 ;; esac
    [ -f "$file" ] && { echo "dev-link is active: just dev-unlink first (the pin is what is bumped)" >&2; exit 1; }
    before=$(pin "$dep")
    [ -n "$before" ] || { echo "Cargo.lock has nothing from $(url "$dep")" >&2; exit 1; }
    head=$(git ls-remote "$(url "$dep")" HEAD | cut -f1)
    [ -n "$head" ] || { echo "$(url "$dep"): no HEAD" >&2; exit 1; }
    # The pin's commit replaced, then checked: a `cargo update -p` would
    # also move unrelated packages within their ranges. When the new commit
    # changes what the packages depend on, Cargo resolves them again.
    cp Cargo.lock Cargo.lock.bump
    sed "s|^\(source = \"git+$(url "$dep")\([?][^#]*\)\{0,1\}#\)$before\"$|\1$head\"|" Cargo.lock.bump >Cargo.lock
    if ! cargo metadata --format-version 1 --locked >/dev/null 2>&1; then
      cp Cargo.lock.bump Cargo.lock
      args=()
      for p in $(packages "$dep"); do args+=(-p "$p"); done
      cargo update "${args[@]}"
    fi
    rm -f Cargo.lock.bump
    after=$(pin "$dep")
    if [ "$before" = "$after" ]; then
      echo "$dep: already at its main's newest commit (${after:0:12})"
    else
      echo "$dep: ${before:0:12} -> ${after:0:12}; commit Cargo.lock"
    fi
    ;;
  status)
    if [ -f "$file" ]; then
      echo "linked: $(sed -n 's/^# link //p' "$file" | tr '\n' ' ')"
      exit 0
    fi
    echo "not linked: Otter and engram as Cargo.lock pins them"
    exit 1
    ;;
  warn)
    if [ -f "$file" ]; then
      {
        printf '\033[33mWARNING: dev-link is active (%s): building against %s.\033[0m\n' "$file" "$(sed -n 's/^# link [a-z]* //p' "$file" | tr '\n' ' ')"
        printf '\033[33mCargo.lock has lost its pins meanwhile: `just dev-unlink` before committing it.\033[0m\n'
      } >&2
    fi
    ;;
  *)
    echo "usage: dev-link.sh link [otter] [engram] | unlink | bump otter|engram | status | warn" >&2
    exit 2
    ;;
esac
