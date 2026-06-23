#!/usr/bin/env bash
# ci-test.sh — emit a fresh project and run the same build, lint, and test
# gates its own CI (reusable-ci.yml) runs, so archetype breakage that only
# surfaces on a real compile is caught here rather than in a spawned repo.
#
# The other emission tests evaluate the flake and exercise the formatters but
# never compile the spawn's crates, so drift between an archetype and the
# foundation library it builds against accumulates silently.  This test closes
# that gap.
#
# The spawn is pointed at the on-disk rust-template it was emitted from rather
# than a published release: a feature branch is ahead of what is published, so
# building against the working tree is the only way to test the archetypes and
# foundation as they actually are.  Only the cargo dependency is redirected —
# the checks run cargo inside this repo's devShell, whose toolchain matches the
# spawn's, so the spawn's own flake never needs to evaluate.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TMPBASE="$(mktemp --directory)"
SPAWN="$TMPBASE/ci-coverage-test"
SPAWN_NAME="ci-coverage-test"
CONFIG="$SCRIPT_DIR/config.json"

# Cleanup mirrors test-formatters.sh: drop the registry entry new-project.sh
# adds to config.json, then remove the temp tree.  Both best-effort so a
# cleanup failure can't mask a real test failure.
cleanup() {
    set +e
    if [[ -f "$CONFIG" ]] && command -v jq >/dev/null 2>&1; then
        jq --arg name "$SPAWN_NAME" \
           'del(.templateSpawns[$name])' \
           "$CONFIG" > "$CONFIG.tmp" && mv "$CONFIG.tmp" "$CONFIG"
    fi
    rm --recursive --force "$TMPBASE"
}
trap cleanup EXIT

# nix is required: the gates run inside `nix develop` for a toolchain that
# matches the spawn's.  Skip cleanly where it is absent (the same posture the
# other emission tests take) so a developer without nix is not blocked.
if ! command -v nix >/dev/null 2>&1; then
    echo "  (skipping CI gate — nix not on PATH)"
    exit 0
fi

echo "Spawning test project (cli + server) at $SPAWN ..."
"$SCRIPT_DIR/new-project.sh" \
    --name "$SPAWN_NAME" \
    --output "$SPAWN" \
    --crates cli,server \
    --description "CI coverage test scratch" \
    > /dev/null

# Redirect the foundation dependency at the on-disk rust-template and relock so
# Cargo.lock pins the working-tree revision instead of the stale placeholder the
# template ships.  `git = "file://..."` is a local git source cargo can resolve
# offline, and `cargo generate-lockfile` rewrites the whole lock around it.
echo "Pointing foundation at the on-disk rust-template ..."
sed --in-place \
    "s|git = \"https://github.com/LoganBarnett/rust-template.git\"|git = \"file://$SCRIPT_DIR\"|" \
    "$SPAWN/Cargo.toml"
( cd "$SPAWN" && nix develop "$SCRIPT_DIR" --command cargo generate-lockfile )

# Run the build-gating CI jobs from reusable-ci.yml against the spawn.  The
# changelog and ABI jobs are skipped: they gate on a published baseline and a
# changelog diff that a day-zero spawn has neither of.  Formatting is covered by
# test-formatters.sh.
run_gate() {
    local label="$1"; shift
    echo "── $label ──"
    if ( cd "$SPAWN" && nix develop "$SCRIPT_DIR" --command "$@" ); then
        echo "  PASS $label"
        return 0
    fi
    echo "  FAIL $label"
    return 1
}

failed=0
run_gate "clippy (warnings denied)" \
    cargo clippy --workspace --all-targets --all-features -- --deny warnings \
    || failed=1
run_gate "test" \
    cargo test --workspace --all-features \
    || failed=1

echo
if [[ "$failed" -eq 0 ]]; then
    echo "Summary: spawn passes the build/lint/test CI gates."
else
    echo "Summary: spawn FAILED a CI gate — an archetype no longer compiles or"
    echo "passes its own CI.  Fix it under template/ (compare against the"
    echo "tests/example-* references), not in a spawned project."
fi
[[ "$failed" -eq 0 ]]
