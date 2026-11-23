# Stitch: day-to-day chores. `just` lists them.

# List the recipes
default:
    @just --list

# visionOS has no prebuilt standard library: it is built with this nightly.
visionos_nightly := "nightly-2026-10-03"

# Regenerate the web client's TypeScript protocol types (design §2.24) from
# the Rust ones (ts-rs, `ts` feature of stitch-node); commit the result.
# Large integers are JSON numbers on the wire, so `number`, not `bigint`.
client-types:
    rm -rf apps/client/src/api
    TS_RS_EXPORT_DIR="{{justfile_directory()}}/apps/client/src/api" TS_RS_LARGE_INT=number \
        cargo test -p stitch-node --features ts --lib export_bindings
    @ls apps/client/src/api | wc -l | xargs echo "types written:"

# A worktree at ../stitch-<name> on a new branch <name>, ready to build: its
# own Rust build, a copy-on-write clone of this checkout's (dependencies
# built, no disk taken until it differs, gone with it)
worktree name base="main":
    #!/usr/bin/env bash
    set -euo pipefail
    main="$(git rev-parse --path-format=absolute --git-common-dir)/.."
    main="$(cd "$main" && pwd)"
    dir="$(dirname "$main")/stitch-{{name}}"
    git -C "$main" worktree add -b "{{name}}" "$dir" "{{base}}"
    # llama.cpp at its pinned commit only (seconds; the main checkout's is
    # shallow, so it cannot lend its objects).
    git -C "$dir" submodule update --init --depth 1
    # Not one shared target directory: artifacts are named by a hash of
    # their checkout's path, so a shared one only grows (710 GB by 2026-10-15).
    if [ -d "$main/target" ]; then
        if [ "$(uname)" = Darwin ]; then
            cp -cR "$main/target" "$dir/target"
        else
            cp -R --reflink=auto "$main/target" "$dir/target"
        fi
    fi
    echo "$dir"

# Uncommitted changes refuse it; an unmerged branch is kept
# The worktree ../stitch-<name> removed, with its build, and its branch once merged
worktree-rm name:
    #!/usr/bin/env bash
    set -euo pipefail
    main="$(git rev-parse --path-format=absolute --git-common-dir)/.."
    main="$(cd "$main" && pwd)"
    dir="$(dirname "$main")/stitch-{{name}}"
    [ -d "$dir" ] || { echo "no worktree at $dir" >&2; exit 1; }
    if [ -n "$(git -C "$dir" status --porcelain)" ]; then
        echo "$dir has uncommitted changes:" >&2
        git -C "$dir" status --short >&2
        exit 1
    fi
    # git removes no worktree with a submodule checked out (llama.cpp): the
    # directory goes, then git forgets it.
    # A fixer's sandbox empties the directory the node made for it, and
    # cannot remove it.
    rm -rf "$dir" 2>/dev/null || [ -z "$(ls -A "$dir")" ]
    git -C "$main" worktree prune
    git -C "$main" branch -d "{{name}}" || echo "branch {{name}} is not merged into the current branch: kept"

# Otter and engram from the checkouts beside this one (../otter, ../engram)
# instead of Cargo.lock's pins, for cross-repo work: an untracked
# .cargo/dev-link.toml (tools/dev-link.sh). `just dev-unlink` before
# committing Cargo.lock; the self-update and `just install` refuse to build
# while it is active
dev-link *deps:
    tools/dev-link.sh link {{deps}}

# Cargo.lock's pins back, exactly as they were before `just dev-link`
dev-unlink:
    tools/dev-link.sh unlink

# Otter's pin in Cargo.lock moved to the newest commit of its main
bump-otter:
    tools/dev-link.sh bump otter

# engram's pin in Cargo.lock moved to the newest commit of its main
bump-engram:
    tools/dev-link.sh bump engram

# The check set to run before committing (AGENTS.md); on macOS, `check-apple` too
check:
    @tools/dev-link.sh warn
    cargo fmt -- --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    @just deny
    @just notices-check
    @{{ if os() == "macos" { "just check-apple" } else { "true" } }}

# The Rust dependencies' advisories, licenses, bans and sources
# (deny.toml; advisories fetch the RustSEC database). Needs
# cargo-deny (`cargo install cargo-deny --locked`); without it, skipped
# outside CI
deny:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cargo-deny >/dev/null; then
        [ -z "${CI-}" ] || { echo "cargo-deny is not installed" >&2; exit 1; }
        echo "deny: skipped, cargo-deny is not installed (cargo install cargo-deny --locked)" >&2
        exit 0
    fi
    cargo deny --locked check advisories licenses bans sources

# Regenerate the third-party notices (notices/: the Rust crates, the web
# client's npm packages, llama.cpp; notices/README.md); commit the result.
# Needs cargo-about (`cargo install cargo-about --locked --features cli`) and bun
notices *parts:
    tools/notices.sh {{parts}}

# Fail when a generated notices file is not what `just notices` gives
# (a part whose tool is missing is skipped outside CI)
notices-check *parts:
    tools/notices.sh --check {{parts}}

# Clippy on iOS (device and simulator), and a check on visionOS
# What the app links (all but the CLI), built for iOS and visionOS
check-apple:
    tools/rustup-ensure.sh targets aarch64-apple-ios aarch64-apple-ios-sim
    tools/rustup-ensure.sh toolchain {{visionos_nightly}} rust-src
    cargo clippy --workspace --exclude stitch-cli --all-targets --locked --target aarch64-apple-ios -- -D warnings
    cargo clippy --workspace --exclude stitch-cli --all-targets --locked --target aarch64-apple-ios-sim -- -D warnings
    cargo +{{visionos_nightly}} check -Zbuild-std --workspace --exclude stitch-cli --all-targets --locked --target aarch64-apple-visionos

# Build and install `stitch` and `stitch-shell-helper` together (release,
# --locked; refused while `just dev-link` is active), staged and renamed
# into ~/.cargo/bin. On macOS `stitch` is
# signed first with an Apple Development identity (or STITCH_SIGN_IDENTITY),
# so the keychain's "Always Allow" outlives rebuilds: an ad-hoc signature
# is the binary's hash, a new app to the keychain each time
install:
    deploy/node/install.sh

# A node of a new version migrates its store on its first start, if it has to.
# The running node sees the new build installed and restarts into it once idle.
# Otter and engram come as Cargo.lock pins them (--locked); refused while
# `just dev-link` is active.
# Pull Stitch and llama.cpp, install: the node restarts into it once idle
update:
    git pull --ff-only
    git submodule update --init
    just install
    @sleep 12
    just start

# Start the node if none runs, and say how it is
start:
    stitch jobs
    stitch node status
    @echo "--- node.log"
    @tail -n 8 "${STITCH_DATA:-$HOME/.stitch}/node.log"

# The node starts its successor itself (never inside the asking shell's
# sandbox). Turns at work stop; detached threads resume.
# Restart the node now (`stitch node restart` alone waits until it is idle)
restart:
    stitch node restart --now
    just start

# It needs `just` and a checkout at ~/devel/stitch; a login shell gives the node a full PATH.
# `just update` on another node, over SSH
update-remote host:
    ssh -t {{host}} '$SHELL -lc "cd ~/devel/stitch && just update"'

# Follow the node's log
log:
    tail -f "${STITCH_DATA:-$HOME/.stitch}/node.log"

# While developing: fmt, clippy and tests of the crates changed against main
# (committed or not), and the web client if it changed
# Fast checks of what changed (against main by default)
quick base="main":
    tools/quick.sh {{base}}

# The hazard guard's eval against the real Decisions API (design §2.28):
# obvious hazards, routine commands, hazards hidden in programs and their
# look-alikes, through the guard. Needs the node's `gemini` key; not in CI.
hazard-eval:
    GEMINI_API_KEY="$(stitch secret get gemini --reveal)" cargo test -p stitch-node --lib hazard::eval -- --ignored --nocapture

# Loose ends' judgement by the decision model against the quick model's, on
# this node's real conversations (read only; `STITCH_EVAL_N`, default 400,
# the latest judgements; rows in /tmp/decide-eval/loose-ends.jsonl). Not in CI.
loose-ends-eval:
    GEMINI_API_KEY="$(stitch secret get gemini --reveal)" cargo test -p stitch-node --lib loose::eval -- --ignored --nocapture

# Build the web client (design §2.24) into apps/client/dist/, which is
# committed and embedded in the node. The protocol version the client says in
# `Request::Hello` is stamped from stitch-node's `VERSION` (crate version and
# `local-N`), so a protocol bump changes the bundle. Type-checks and runs the
# client's tests (`bun test`, apps/client/test) first. Needs bun (~/.bun/bin).
client:
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="$HOME/.bun/bin:$PATH"
    ver="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"
    loc="$(grep -o '"/local-[0-9]*"' crates/stitch-node/src/lib.rs | head -1 | tr -d '"')"
    cd apps/client
    printf '// Generated by `just client` from stitch-node'"'"'s VERSION; do not edit.\nexport const CLIENT_VERSION = "%s%s";\n' "$ver" "$loc" > src/version.ts
    bun install --frozen-lockfile
    bunx tsc --noEmit
    bun test --bail
    rm -rf dist && mkdir dist
    bun build src/main.tsx --minify --target browser --format iife --outdir dist --entry-naming 'app.[ext]'
    # Space, the map in 3D (three.js): its own ES module, loaded by `import()` the first time 3D is shown (design §2.24).
    bun build src/map/space/entry.ts --minify --target browser --format esm --outdir dist --entry-naming 'space.[ext]'
    cp src/index.html dist/index.html
    # The built-in dashboard (design §2.10): the home view on `stitch.live`.
    bun build src/dashboard/main.tsx --minify --target browser --format iife --outdir dist/dashboard --entry-naming 'app.[ext]'
    cp src/dashboard/index.html dist/dashboard/index.html
    ls dist dist/dashboard

# Fail when rebuilding the client gives other files than the committed dist/.
client-check: client
    git diff --exit-code -- apps/client/dist apps/client/src/version.ts
    @test -z "$(git ls-files --others --exclude-standard apps/client)" || { echo "untracked client files"; exit 1; }
