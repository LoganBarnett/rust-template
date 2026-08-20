#!/usr/bin/env bash
# Unit test for the Dependabot auto-merge file allowlist
# (.github/actions/dependabot-allowlist/allowlist.sh).
#
# The auto-merge flow proceeds only when a bump touches files Dependabot is
# expected to edit; anything else signals a human touched the PR and the flow
# stays out of it.  The predicate is pure — it reads a newline-delimited file
# list on stdin and prints the paths outside the allowlist — so this test drives
# every branch by feeding crafted file lists, with no live PR required (the same
# injectable-input approach the review-stop crate's gate tests use).
#
# The load-bearing case is a workspace-member manifest
# (crates/<name>/Cargo.toml): the original allowlist anchored Cargo.toml to the
# repo root, so every member-crate bump read as an unexpected file and was
# skipped, stranding most bumps in a workspace project.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALLOWLIST="$SCRIPT_DIR/.github/actions/dependabot-allowlist/allowlist.sh"

fail=0

# assert_allowed NAME FILES... — every path is on the allowlist (no output).
assert_allowed() {
  local name=$1
  shift
  local out
  out=$(printf '%s\n' "$@" | "$ALLOWLIST" CHANGELOG.org)
  if [ -n "$out" ]; then
    echo "FAIL: $name — expected all allowed, but flagged:"
    printf '  %s\n' "$out"
    fail=1
  else
    echo "ok: $name"
  fi
}

# assert_flagged NAME EXPECTED FILES... — EXPECTED must appear in the output.
assert_flagged() {
  local name=$1 expected=$2
  shift 2
  local out
  out=$(printf '%s\n' "$@" | "$ALLOWLIST" CHANGELOG.org)
  if printf '%s\n' "$out" \
      | grep --quiet --line-regexp --fixed-strings "$expected"; then
    echo "ok: $name"
  else
    echo "FAIL: $name — expected '$expected' to be flagged, got:"
    printf '  %s\n' "${out:-<none>}"
    fail=1
  fi
}

echo "== allowed (legitimate Dependabot bumps) =="
assert_allowed "root manifest bump"        "Cargo.toml" "Cargo.lock"
assert_allowed "workspace-member bump"     "Cargo.lock" "crates/foundation/Cargo.toml"
assert_allowed "two member manifests"      "Cargo.lock" \
  "crates/foundation/Cargo.toml" "crates/compliance-lib/Cargo.toml"
assert_allowed "github-actions bump"       ".github/workflows/ci.yml"
assert_allowed "bump plus added changelog" "Cargo.lock" \
  "crates/foundation/Cargo.toml" "CHANGELOG.org"

echo "== flagged (a human touched the PR) =="
assert_flagged "source edit" "crates/foundation/src/lib.rs" \
  "Cargo.lock" "crates/foundation/src/lib.rs"
assert_flagged "readme edit" "README.org" \
  "Cargo.toml" "README.org"
assert_flagged "sneaky prefix" "evilCargo.toml" \
  "evilCargo.toml"
assert_flagged "manifest with suffix" "Cargo.toml.bak" \
  "Cargo.toml.bak"

if [ "$fail" -ne 0 ]; then
  echo "FAILED"
  exit 1
fi
echo "PASSED"
