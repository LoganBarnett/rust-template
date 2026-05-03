#!/usr/bin/env bash
# compliance-check.sh — verify structural correctness of spawned repos.
#
# Reads config.json and runs structural checks against each non-archived
# spawn whose directory exists on disk.  Skips temp directories that no
# longer exist (e.g. from test runs).
#
# Usage:
#   ./compliance-check.sh              # check all spawns
#   ./compliance-check.sh my-project   # check a single spawn
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG="$SCRIPT_DIR/config.json"

if [[ ! -f "$CONFIG" ]]; then
    echo "No config.json found at $CONFIG" >&2
    exit 1
fi

# Counters.
total_pass=0
total_fail=0
total_skip=0
projects_checked=0
projects_skipped=0

# Colors (if stdout is a terminal).
if [[ -t 1 ]]; then
    GREEN='\033[0;32m'
    RED='\033[0;31m'
    YELLOW='\033[0;33m'
    BOLD='\033[1m'
    RESET='\033[0m'
else
    GREEN='' RED='' YELLOW='' BOLD='' RESET=''
fi

pass() {
    printf "  ${GREEN}PASS${RESET} %s\n" "$1"
    total_pass=$((total_pass + 1))
}

fail() {
    printf "  ${RED}FAIL${RESET} %s\n" "$1"
    total_fail=$((total_fail + 1))
}

skip_check() {
    printf "  ${YELLOW}SKIP${RESET} %s\n" "$1"
}

# ── Individual checks ────────────────────────────────────────────────

check_required_files() {
    local dir="$1"
    local required_files=(
        "CHANGELOG.org"
        ".envrc"
        "rustfmt.toml"
        "Cargo.toml"
        "flake.nix"
        "flake.lock"
        ".github/workflows/ci.yml"
    )
    for f in "${required_files[@]}"; do
        if [[ -f "$dir/$f" ]]; then
            pass "required file exists: $f"
        else
            fail "required file missing: $f"
        fi
    done
}

check_provenance_file() {
    local dir="$1"
    if [[ -f "$dir/rust-template.json" ]]; then
        if jq empty "$dir/rust-template.json" 2>/dev/null; then
            pass "rust-template.json is valid JSON"
        else
            fail "rust-template.json is not valid JSON"
        fi
    else
        fail "rust-template.json provenance file missing"
    fi
}

check_foundation_dependency() {
    local dir="$1"
    if grep -q 'rust-template-foundation' "$dir/Cargo.toml" 2>/dev/null; then
        pass "foundation dependency in workspace Cargo.toml"
    else
        fail "foundation dependency missing from workspace Cargo.toml"
    fi
}

check_foundation_features() {
    local dir="$1" name="$2" crates="$3"
    # Server crates should use the "auth" feature.
    if [[ "$crates" == *server* ]]; then
        if grep -rq 'features.*=.*\[.*"auth"' "$dir/crates/" 2>/dev/null; then
            pass "foundation auth feature enabled (server crate)"
        else
            fail "foundation auth feature not found (server crate expected)"
        fi
    fi
    # CLI crates should use the "cli" feature.
    if [[ "$crates" == *cli* ]]; then
        if grep -rq 'features.*=.*\[.*"cli"' "$dir/crates/" 2>/dev/null; then
            pass "foundation cli feature enabled (cli crate)"
        else
            fail "foundation cli feature not found (cli crate expected)"
        fi
    fi
}

check_ci_reusable_workflows() {
    local dir="$1"
    local ci="$dir/.github/workflows/ci.yml"
    if [[ ! -f "$ci" ]]; then
        skip_check "CI workflow missing, cannot check reusable workflow refs"
        return
    fi
    if grep -q 'LoganBarnett/rust-template/' "$ci"; then
        pass "CI calls reusable workflows from rust-template"
    else
        fail "CI does not reference LoganBarnett/rust-template/ workflows"
    fi
}

check_stale_literals() {
    local dir="$1" name="$2"
    # Search for "rust-template" in source files, excluding:
    # - rust-template-foundation references (expected)
    # - rust-template.json (provenance file)
    # - flake.lock (Nix lock file with hashes)
    # - .git directory
    # - target directory (build artifacts)
    local stale
    stale=$(grep -r --include='*.rs' --include='*.toml' --include='*.nix' \
                --include='*.yml' --include='*.yaml' --include='*.json' \
                -l 'rust-template' "$dir" 2>/dev/null \
        | grep -v 'rust-template-foundation' \
        | grep -v 'rust-template\.json' \
        | grep -v 'flake\.lock' \
        | grep -v '/\.git/' \
        | grep -v '/target/' \
        | grep -v 'LoganBarnett/rust-template' \
        || true)

    if [[ -z "$stale" ]]; then
        pass "no stale rust-template literals"
    else
        # Filter further: check if the remaining files actually contain
        # "rust-template" without the foundation or GitHub URL context.
        local real_stale=""
        for f in $stale; do
            # Get lines with rust-template that are NOT foundation refs,
            # NOT GitHub URLs, and NOT flake input refs.
            if grep -q 'rust-template' "$f" 2>/dev/null \
                && ! grep 'rust-template' "$f" \
                    | grep -qE '(rust-template-foundation|LoganBarnett/rust-template|foundation.*rust-template)'; then
                real_stale="$real_stale $f"
            fi
        done
        if [[ -z "$real_stale" ]]; then
            pass "no stale rust-template literals"
        else
            fail "stale rust-template literals in:$real_stale"
        fi
    fi
}

check_foundation_flake_input() {
    local dir="$1"
    local flake="$dir/flake.nix"
    if [[ ! -f "$flake" ]]; then
        skip_check "flake.nix missing, cannot check foundation input"
        return
    fi
    if grep -q 'foundation.*LoganBarnett/rust-template' "$flake" \
        || grep -q 'LoganBarnett/rust-template' "$flake"; then
        pass "flake.nix references rust-template as foundation input"
    else
        # Older spawns may not have the foundation input yet.
        skip_check "flake.nix does not reference foundation input (may be pre-helper)"
    fi
}

# ── Main loop ────────────────────────────────────────────────────────

filter_project="${1:-}"

# Get list of project names.
projects=$(jq -r '.templateSpawns | keys[]' "$CONFIG")

for project in $projects; do
    # Filter to single project if specified.
    if [[ -n "$filter_project" && "$project" != "$filter_project" ]]; then
        continue
    fi

    archived=$(jq -r ".templateSpawns[\"$project\"].archived" "$CONFIG")
    dir=$(jq -r ".templateSpawns[\"$project\"].dir" "$CONFIG")
    crates=$(jq -r ".templateSpawns[\"$project\"].args.crates" "$CONFIG")

    if [[ "$archived" == "true" ]]; then
        printf "${YELLOW}SKIP${RESET} %s (archived)\n" "$project"
        projects_skipped=$((projects_skipped + 1))
        continue
    fi

    if [[ ! -d "$dir" ]]; then
        printf "${YELLOW}SKIP${RESET} %s (directory missing: %s)\n" "$project" "$dir"
        projects_skipped=$((projects_skipped + 1))
        continue
    fi

    printf "\n${BOLD}Checking: %s${RESET} (%s)\n" "$project" "$dir"
    projects_checked=$((projects_checked + 1))

    check_required_files "$dir"
    check_provenance_file "$dir"
    check_foundation_dependency "$dir"
    check_foundation_features "$dir" "$project" "$crates"
    check_ci_reusable_workflows "$dir"
    check_stale_literals "$dir" "$project"
    check_foundation_flake_input "$dir"
done

# ── Summary ──────────────────────────────────────────────────────────

echo ""
echo "═══════════════════════════════════════════"
printf "Projects checked: %d  Skipped: %d\n" "$projects_checked" "$projects_skipped"
printf "Checks:  ${GREEN}%d passed${RESET}  ${RED}%d failed${RESET}\n" \
    "$total_pass" "$total_fail"
echo "═══════════════════════════════════════════"

if [[ "$total_fail" -gt 0 ]]; then
    exit 1
fi
