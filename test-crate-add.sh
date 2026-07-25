#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=script-common.sh
source "$SCRIPT_DIR/script-common.sh"
TMPBASE="$(mktemp -d)"
trap 'rm -rf "$TMPBASE"' EXIT

PASS=0
FAIL=0

# Cargo check requires nix AND network access to the foundation crate.
# Set TEST_CARGO_CHECK=1 to enable cargo check assertions.
RUN_CARGO_CHECK=false
if [[ "${TEST_CARGO_CHECK:-}" == "1" ]] && command -v nix &>/dev/null; then
    RUN_CARGO_CHECK=true
fi

run_test() {
    local name="$1"; shift
    if "$@"; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name"
        FAIL=$((FAIL + 1))
    fi
}

assert_dir_exists() {
    if [[ ! -d "$1" ]]; then
        echo "  assertion failed: directory does not exist: $1" >&2
        return 1
    fi
}

assert_dir_not_exists() {
    if [[ -d "$1" ]]; then
        echo "  assertion failed: directory should not exist: $1" >&2
        return 1
    fi
}

assert_file_exists() {
    if [[ ! -f "$1" ]]; then
        echo "  assertion failed: file does not exist: $1" >&2
        return 1
    fi
}

assert_file_contains() {
    if ! grep -q "$2" "$1" 2>/dev/null; then
        echo "  assertion failed: '$1' does not contain '$2'" >&2
        return 1
    fi
}

assert_file_not_contains() {
    if grep -q "$2" "$1" 2>/dev/null; then
        echo "  assertion failed: '$1' should not contain '$2'" >&2
        return 1
    fi
}

# Assert that no file in the directory tree contains the given pattern,
# optionally excluding lines that match an exception pattern.
assert_no_occurrence() {
    local dir="$1" pattern="$2" exception="${3:-}"
    local matches
    if [[ -n "$exception" ]]; then
        matches=$(grep -rl "$pattern" "$dir" 2>/dev/null \
            | xargs grep -l "$pattern" 2>/dev/null \
            | while IFS= read -r f; do
                if grep "$pattern" "$f" | grep -qv "$exception"; then
                    echo "$f"
                fi
            done || true)
    else
        matches=$(grep -rl "$pattern" "$dir" 2>/dev/null || true)
    fi
    if [[ -n "$matches" ]]; then
        echo "  assertion failed: pattern '$pattern' found in:" >&2
        echo "$matches" | head -5 | sed 's/^/    /' >&2
        return 1
    fi
}

assert_exit_code() {
    local expected="$1"; shift
    local actual=0
    "$@" || actual=$?
    if [[ "$actual" -ne "$expected" ]]; then
        echo "  assertion failed: expected exit code $expected, got $actual" >&2
        return 1
    fi
}

# Assert that the emitted project's flake.nix evaluates cleanly against
# the *current* state of the rust-template foundation library.
#
# Spawned projects normally pin foundation via flake.lock to a historical
# commit, so they may continue to eval even when the template's emitted
# flake.nix drifts out of sync with foundation's `lib.mkRustPackages` API.
# That drift only bites users when they run `nix flake update`.  Override
# foundation to the rust-template git tree under test so this assertion
# catches the drift before it ships.
#
# Uses `git+file:` (not `path:`) so the import is scoped to tracked
# files only — target/ and .direnv/ are gitignored and would otherwise
# bloat the store import with gigabytes of build artifacts.  Uncommitted
# changes to tracked files are still picked up (with a "Git tree is
# dirty" warning from nix), so iterative test-driven changes work.
#
# Skips cleanly on hosts without nix; CI installs nix and runs this.
assert_flake_eval() {
    local dir="$1"
    if ! command -v nix &>/dev/null; then
        echo "  (skipping flake-eval — nix not on PATH)"
        return 0
    fi
    local nix_args=(
        --extra-experimental-features 'nix-command flakes'
    )
    local show_args=(
        --override-input foundation "git+file://$SCRIPT_DIR"
        --no-update-lock-file
        "$dir"
    )
    if ! nix "${nix_args[@]}" flake show "${show_args[@]}" \
            >/dev/null 2>&1; then
        echo "  assertion failed: nix flake show failed for $dir" >&2
        nix "${nix_args[@]}" flake show "${show_args[@]}" 2>&1 \
            | sed 's/^/    /' | head -30 >&2
        return 1
    fi
}

# Assert every `-aarch64-darwin` output the emitted spawn exposes carries the
# expected `appleSdkWired` marker: `true` for a framework-linking (auth) spawn,
# `false` otherwise.  This reads back the passthru mkDarwinCrossPackages
# attaches to each darwin package, proving the emitted flake's flag-driven
# wiring actually reaches the SDK argument — the class of surprise the repo's
# own fixture cannot catch, since it is handed `appleSdk` directly.  Evaluated
# for x86_64-linux
# (where the darwin cross outputs exist) against the on-disk foundation, from
# whatever host runs the test; eval-only, so no darwin build is realized.
assert_apple_sdk_wired() {
    local dir="$1" expected="$2"
    if ! command -v nix &>/dev/null; then
        echo "  (skipping appleSdk-wired eval — nix not on PATH)"
        return 0
    fi
    # Every `-aarch64-darwin` package must report the expected marker, and there
    # must be at least one (an empty match would vacuously pass).
    local apply="p: let names = builtins.filter (n: builtins.match \".*-aarch64-darwin\" n != null) (builtins.attrNames p); in names != [] && builtins.all (n: p.\${n}.appleSdkWired == $expected) names"
    local got
    got=$(nix \
        --extra-experimental-features 'nix-command flakes' \
        eval \
        --override-input foundation "git+file://$SCRIPT_DIR" \
        --no-update-lock-file \
        "$dir#packages.x86_64-linux" \
        --apply "$apply" 2>/dev/null)
    if [[ "$got" != "true" ]]; then
        echo "  assertion failed: expected all -aarch64-darwin appleSdkWired == $expected for $dir (eval result: '${got:-<empty>}')" >&2
        return 1
    fi
}

# Assert that a freshly emitted project passes every compliance check — that
# "the template emits a compliant project" actually holds.
#
# Registers the emission as a single spawn in a throwaway config.json (supplying
# its crate set so the role-conditional checks run), then runs the compliance
# checker against it and fails on any `fail` or `error` outcome.  Nothing is
# tolerated: the foundation pin checks skip on a fresh emission (the template
# Cargo.lock has no resolved foundation git rev), and pin hygiene is a
# template-maintenance concern tracked as its own task.
#
# Gated on the checker having been built (which is gated on cargo); hosts
# without a Rust toolchain skip cleanly, as with the cargo-check assertion.
assert_compliant() {
    local dir="$1" crates="$2" public="${3:-false}"
    if [[ -z "$COMPLIANCE_BIN" || ! -x "$COMPLIANCE_BIN" ]]; then
        echo "  (skipping compliance check — checker not built)"
        return 0
    fi
    local config="$TMPBASE/compliance-registry-$(basename "$dir").json"
    jq -n --arg dir "$dir" --arg crates "$crates" --argjson public "$public" \
        '{templateSpawns: {emission: {dir: $dir, archived: false, args: {crates: $crates, public: $public}}}}' \
        > "$config"
    # The checker's exit code is authoritative: non-zero iff a check failed or
    # errored (or the run could not start at all).  Trust it for the pass/fail
    # decision; the JSON report is only for showing *which* checks failed.
    local report exit_code=0
    report=$("$COMPLIANCE_BIN" \
        --registry "$config" \
        --manifest "$SCRIPT_DIR/compliance-checks.toml" \
        --template-dir "$SCRIPT_DIR" \
        --format json 2>/dev/null) || exit_code=$?
    if [[ "$exit_code" -ne 0 ]]; then
        echo "  assertion failed: fresh emission is not compliant:" >&2
        printf '%s' "$report" \
            | jq -r '.spawns[].checks[]
                     | select(.status == "fail" or .status == "error")
                     | "    [\(.id)] \(.detail // "")"' 2>/dev/null >&2 \
            || echo "    (checker exited $exit_code without a JSON report)" >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# Test 1: new-project.sh with cli+server (default)
# ---------------------------------------------------------------------------
test_new_project_default() {
    local dir="$TMPBASE/test-default"
    "$SCRIPT_DIR/new-project.sh" \
        --name test-app \
        --output "$dir" \
        --description "Test application"

    assert_dir_exists "$dir/crates/lib"
    assert_dir_exists "$dir/crates/cli"
    assert_dir_exists "$dir/crates/server"

    # Cargo.toml members list contains all three crates.
    assert_file_contains "$dir/Cargo.toml" '"crates/lib"'
    assert_file_contains "$dir/Cargo.toml" '"crates/cli"'
    assert_file_contains "$dir/Cargo.toml" '"crates/server"'

    # flake.nix has sentinel blocks for binary crates.
    assert_file_contains "$dir/flake.nix" '# CRATE:cli:begin'
    assert_file_contains "$dir/flake.nix" '# CRATE:cli:end'
    assert_file_contains "$dir/flake.nix" '# CRATE:server:begin'
    assert_file_contains "$dir/flake.nix" '# CRATE:server:end'

    # No rust-template literals remain, except: the foundation crate
    # (rust-template-foundation), any `<owner>/rust-template` git/flake/file
    # reference (owner-agnostic so forks are not penalised, matching the
    # no-stale-rust-template-literals compliance check), and the
    # rust-template.json manifest, which keeps its literal filename in a spawn.
    assert_no_occurrence "$dir" "rust-template" "rust-template-foundation\|/rust-template\|rust-template\.json"

    # Package names match test-app-{cli,server,lib}.
    assert_file_contains "$dir/crates/cli/Cargo.toml" 'name = "test-app-cli"'
    assert_file_contains "$dir/crates/server/Cargo.toml" 'name = "test-app-server"'
    assert_file_contains "$dir/crates/lib/Cargo.toml" 'name = "test-app-lib"'

    # A server spawn enables foundation auth, which links Apple frameworks, so
    # the emission sets the apple-frameworks opt-in that wires the darwin SDK.
    assert_file_contains "$dir/rust-template.json" '"apple-frameworks": true'

    # Point foundation at the on-disk template and relock so every check runs
    # against the template that emitted this spawn, not the stale revision the
    # emitted flake ships pinned.
    localize_foundation "$dir" || return 1

    # Compliance: a fresh emission must pass every check.
    assert_compliant "$dir" "cli,server" || return 1

    # Flake-eval assertion: catches API drift between template and foundation.
    assert_flake_eval "$dir" || return 1

    # An auth spawn's darwin cross outputs must actually receive the Apple SDK —
    # proves the emitted flag-driven wiring reaches mkDarwinCrossPackages.
    assert_apple_sdk_wired "$dir" true || return 1

    # Cargo check (nix-gated).
    if [[ "$RUN_CARGO_CHECK" == true ]]; then
        (cd "$dir" && nix develop --command cargo check) || return 1
    else
        echo "  (skipping cargo check — set TEST_CARGO_CHECK=1 to enable)"
    fi
}

# ---------------------------------------------------------------------------
# Test 2: new-project.sh with cli only
# ---------------------------------------------------------------------------
test_new_project_cli_only() {
    local dir="$TMPBASE/test-cli-only"
    "$SCRIPT_DIR/new-project.sh" \
        --name test-app \
        --output "$dir" \
        --crates cli

    assert_dir_exists "$dir/crates/cli"
    assert_dir_not_exists "$dir/crates/server"
    assert_dir_exists "$dir/crates/lib"

    assert_file_contains "$dir/Cargo.toml" '"crates/cli"'
    assert_file_not_contains "$dir/Cargo.toml" '"crates/server"'

    assert_file_contains "$dir/flake.nix" '# CRATE:cli:begin'
    assert_file_not_contains "$dir/flake.nix" '# CRATE:server:begin'

    # The publish workflow ships in private spawns too, not just public ones.
    assert_file_exists "$dir/.github/workflows/publish.yml"

    # Private spawns keep an empty publish list — nothing reaches crates.io.
    assert_file_not_contains "$dir/crates/lib/Cargo.toml" 'crates-io'

    # A cli/lib-only spawn links no Apple frameworks, so the opt-in stays false
    # and no unfree Apple SDK licence is accepted.
    assert_file_contains "$dir/rust-template.json" '"apple-frameworks": false'

    localize_foundation "$dir" || return 1
    assert_compliant "$dir" "cli" || return 1

    assert_flake_eval "$dir" || return 1

    # A cli/lib-only spawn links no frameworks, so its darwin outputs must
    # report the SDK unwired — the licence-free path.
    assert_apple_sdk_wired "$dir" false || return 1
}

# ---------------------------------------------------------------------------
# Test 3: new-project.sh with server only
# ---------------------------------------------------------------------------
test_new_project_server_only() {
    local dir="$TMPBASE/test-server-only"
    "$SCRIPT_DIR/new-project.sh" \
        --name test-app \
        --output "$dir" \
        --crates server

    assert_dir_exists "$dir/crates/server"
    assert_dir_not_exists "$dir/crates/cli"
    assert_dir_exists "$dir/crates/lib"

    assert_file_contains "$dir/Cargo.toml" '"crates/server"'
    assert_file_not_contains "$dir/Cargo.toml" '"crates/cli"'

    assert_file_contains "$dir/flake.nix" '# CRATE:server:begin'
    assert_file_not_contains "$dir/flake.nix" '# CRATE:cli:begin'

    # A server spawn enables foundation auth (framework-linking), so it carries
    # the apple-frameworks opt-in.
    assert_file_contains "$dir/rust-template.json" '"apple-frameworks": true'

    localize_foundation "$dir" || return 1
    assert_compliant "$dir" "server" || return 1
}

# ---------------------------------------------------------------------------
# Test 3b: new-project.sh --public — exercises the public-only behavior, namely
# pointing the library crate's publish destination at crates.io.  The publish
# workflow itself ships in every spawn, so its presence is asserted in the
# private emissions too.
# ---------------------------------------------------------------------------
test_new_project_public() {
    local dir="$TMPBASE/test-public"
    "$SCRIPT_DIR/new-project.sh" \
        --name test-app \
        --output "$dir" \
        --public

    # The publish workflow ships in every spawn; public toggles its destination,
    # not its presence.
    assert_file_exists "$dir/.github/workflows/publish.yml"

    # Public spawns point the library's publish destination at crates.io.
    assert_file_contains "$dir/crates/lib/Cargo.toml" 'crates-io'

    # Point foundation at the on-disk rust-template and relock so the foundation
    # pin checks run against a real, agreeing revision rather than skipping on
    # the stale path placeholder the template ships.  This is the one spawn that
    # exercises the pins; they are an emission-invariant property, so verifying
    # them here covers the others.
    localize_foundation "$dir" || return 1
    assert_file_contains "$dir/Cargo.lock" 'source = "git+file://'

    # Compliance with public = true so the gated publish checks actually run.
    assert_compliant "$dir" "cli,server" true || return 1
}

# ---------------------------------------------------------------------------
# Test 4: Standalone crate-add.sh — add server to cli-only project
# ---------------------------------------------------------------------------
test_add_server_to_cli_project() {
    local dir="$TMPBASE/test-add-server"
    "$SCRIPT_DIR/new-project.sh" \
        --name test-app \
        --output "$dir" \
        --crates cli

    # Verify server does not exist yet.
    assert_dir_not_exists "$dir/crates/server"

    # Add server crate.
    "$SCRIPT_DIR/crate-add.sh" \
        --type server \
        --project-dir "$dir"

    assert_dir_exists "$dir/crates/server"
    assert_file_contains "$dir/crates/server/Cargo.toml" 'name = "test-app-server"'
    assert_file_contains "$dir/Cargo.toml" '"crates/server"'
    assert_file_contains "$dir/flake.nix" '# CRATE:server:begin'

    # Workspace deps for server (axum, tokio) are present.
    assert_file_contains "$dir/Cargo.toml" 'axum'
    assert_file_contains "$dir/Cargo.toml" 'tokio'

    # Cargo check (nix-gated).
    if [[ "$RUN_CARGO_CHECK" == true ]]; then
        (cd "$dir" && nix develop --command cargo check) || return 1
    else
        echo "  (skipping cargo check — set TEST_CARGO_CHECK=1 to enable)"
    fi
}

# ---------------------------------------------------------------------------
# Test 5: Custom crate name
# ---------------------------------------------------------------------------
test_custom_crate_name() {
    local dir="$TMPBASE/test-custom-name"
    "$SCRIPT_DIR/new-project.sh" \
        --name test-app \
        --output "$dir" \
        --crates cli

    "$SCRIPT_DIR/crate-add.sh" \
        --type cli \
        --project-dir "$dir" \
        --name worker

    # Directory is crates/worker/, not crates/cli/ (a second one).
    assert_dir_exists "$dir/crates/worker"

    # Package name is test-app-worker.
    assert_file_contains "$dir/crates/worker/Cargo.toml" 'name = "test-app-worker"'

    # Binary name is test-app-worker.
    assert_file_contains "$dir/crates/worker/Cargo.toml" 'name = "test-app-worker"'

    # Rust module name uses underscores.
    # The cli template does not have a [lib] section, so check the [[bin]] section.
    assert_file_not_contains "$dir/crates/worker/Cargo.toml" 'rust_template'

    # flake.nix has CRATE:worker sentinel (not CRATE:cli duplicated).
    assert_file_contains "$dir/flake.nix" '# CRATE:worker:begin'
    assert_file_contains "$dir/flake.nix" '# CRATE:worker:end'

    # Config file lookup uses project name, not crate name.
    if grep -q 'find_config_file' "$dir/crates/worker/src/config.rs" 2>/dev/null; then
        assert_file_contains "$dir/crates/worker/src/config.rs" 'find_config_file("test-app"'
    fi
}

# ---------------------------------------------------------------------------
# Test 6: Duplicate crate rejection
# ---------------------------------------------------------------------------
test_duplicate_crate_rejection() {
    local dir="$TMPBASE/test-duplicate"
    "$SCRIPT_DIR/new-project.sh" \
        --name test-app \
        --output "$dir" \
        --crates cli

    # First add should succeed (exit 0).
    "$SCRIPT_DIR/crate-add.sh" \
        --type server \
        --project-dir "$dir" || return 1

    # Second add of the same crate should fail with exit code 2.
    assert_exit_code 2 \
        "$SCRIPT_DIR/crate-add.sh" \
        --type server \
        --project-dir "$dir"
}

# ---------------------------------------------------------------------------
# Test 7: Foundation refs preserved
# ---------------------------------------------------------------------------
test_foundation_refs_preserved() {
    local dir="$TMPBASE/test-foundation"
    "$SCRIPT_DIR/new-project.sh" \
        --name test-app \
        --output "$dir"

    # rust-template-foundation should appear in Cargo.toml deps.
    assert_file_contains "$dir/Cargo.toml" 'rust-template-foundation'

    # The mangled form (test-app-foundation) should NOT appear anywhere.
    assert_no_occurrence "$dir" "test-app-foundation"
}

# Build the compliance checker once so each emission test can run it against
# its output.  Gated on cargo, mirroring the cargo-check gating above.
COMPLIANCE_BIN=""
if command -v cargo &>/dev/null; then
    echo "Building the compliance checker..."
    if cargo build --quiet --manifest-path "$SCRIPT_DIR/Cargo.toml" \
            --package rust-template-compliance-cli; then
        COMPLIANCE_BIN="$(cargo metadata --format-version 1 --no-deps \
            --manifest-path "$SCRIPT_DIR/Cargo.toml" 2>/dev/null \
            | jq -r '.target_directory')/debug/rust-template-compliance-cli"
    else
        echo "  (compliance checker failed to build — checks will skip)" >&2
    fi
fi

# ---------------------------------------------------------------------------
# Run all tests.
# ---------------------------------------------------------------------------
echo "Running crate-add integration tests..."
echo ""

run_test "new-project-default" test_new_project_default
run_test "new-project-cli-only" test_new_project_cli_only
run_test "new-project-server-only" test_new_project_server_only
run_test "new-project-public" test_new_project_public
run_test "add-server-to-cli-project" test_add_server_to_cli_project
run_test "custom-crate-name" test_custom_crate_name
run_test "duplicate-crate-rejection" test_duplicate_crate_rejection
run_test "foundation-refs-preserved" test_foundation_refs_preserved

echo ""
echo "$PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
