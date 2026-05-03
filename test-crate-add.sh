#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
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

    # No rust-template literals remain (except foundation refs and workflow refs).
    assert_no_occurrence "$dir" "rust-template" "rust-template-foundation\|LoganBarnett/rust-template"

    # Package names match test-app-{cli,server,lib}.
    assert_file_contains "$dir/crates/cli/Cargo.toml" 'name = "test-app-cli"'
    assert_file_contains "$dir/crates/server/Cargo.toml" 'name = "test-app-server"'
    assert_file_contains "$dir/crates/lib/Cargo.toml" 'name = "test-app-lib"'

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

# ---------------------------------------------------------------------------
# Run all tests.
# ---------------------------------------------------------------------------
echo "Running crate-add integration tests..."
echo ""

run_test "new-project-default" test_new_project_default
run_test "new-project-cli-only" test_new_project_cli_only
run_test "new-project-server-only" test_new_project_server_only
run_test "add-server-to-cli-project" test_add_server_to_cli_project
run_test "custom-crate-name" test_custom_crate_name
run_test "duplicate-crate-rejection" test_duplicate_crate_rejection
run_test "foundation-refs-preserved" test_foundation_refs_preserved

echo ""
echo "$PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
