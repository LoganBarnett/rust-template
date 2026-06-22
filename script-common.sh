#!/usr/bin/env bash
# Shared helpers for the generation scripts (new-project.sh, crate-add.sh) and
# the emission test scripts (test-crate-add.sh, test-formatters.sh).

# Detect whether the sed in PATH is GNU (supports -i without an extension
# argument) or BSD (requires -i '').
if sed --version 2>/dev/null | grep -q GNU; then
    sed_inplace() { sed -i "$@"; }
else
    sed_inplace() { sed -i '' "$@"; }
fi

# Point a spawn's foundation at the on-disk rust-template it was emitted from —
# both the cargo dependency and the flake input — and relock both so their
# pinned foundation revisions reference the working tree under test.  A feature
# branch is ahead of what is published, so a spawn left at the github URL pins a
# published revision that lags the local HEAD: the pin checks then trip, and the
# emitted flake resolves foundation library functions that only exist on the
# branch (e.g. mkMuslPackages) against the older published lib that lacks them.
# Pointing both edges at the local checkout makes them agree on, and match, the
# revision actually being tested.  Callers set SCRIPT_DIR to the rust-template
# checkout before invoking this.
#
# Best-effort: without nix and cargo the spawn keeps its github refs and the pin
# checks stay dormant — the same posture the flake-eval assertion takes.  On a
# clean checkout (CI) the relock records a concrete revision and the pin checks
# are active; on a dirty working tree nix may omit the flake revision, which
# leaves the pin checks skipped rather than failing.
localize_foundation() {
    local dir="$1"
    if ! command -v nix &>/dev/null || ! command -v cargo &>/dev/null; then
        return 0
    fi
    # Owner-agnostic ([^/]* for the owner) so a fork's emitted refs match too.
    sed_inplace \
        "s|git = \"https://github.com/[^/]*/rust-template.git\"|git = \"file://$SCRIPT_DIR\"|" \
        "$dir/Cargo.toml"
    sed_inplace \
        "s|foundation.url = \"github:[^/]*/rust-template\"|foundation.url = \"git+file://$SCRIPT_DIR\"|" \
        "$dir/flake.nix"
    if ! ( cd "$dir" && cargo generate-lockfile \
            && nix flake update foundation ); then
        echo "  localize_foundation: relock failed for $dir" >&2
        return 1
    fi
}
