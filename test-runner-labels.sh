#!/usr/bin/env bash
# Guard against a floating runner label in this repo's workflows.
#
# GitHub redefines the `-latest` runner labels on its own schedule (it has
# announced `ubuntu-latest` will move from Ubuntu 24.04 to 26.04 in October
# 2026), so a workflow that floats on one changes image under every branch at
# once, with no pull request to show for it.  The workflows pin a versioned
# label instead, and the scheduled dependency bump advances the pin through a
# CI-gated pull request (see .github/workflows/reusable-dependency-bump.yml).
# Every spawned project runs on these same workflows via `@main`, so a
# floating label here floats for the whole fleet.  This test fails on any
# `-latest` runner label that sneaks back in.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# `grep` exits 1 when nothing matches, which is the passing outcome here; 0
# means a floating label, and anything else means grep could not scan the
# tree, which must not pass as "nothing found".
floating=$(grep --recursive --line-number --extended-regexp \
  '\b(ubuntu|windows|macos)-latest\b' .github/workflows) && status=0 || status=$?

case $status in
  0)
    echo "FAIL: floating runner labels under .github/workflows:"
    printf '  %s\n' "$floating"
    exit 1
    ;;
  1)
    echo "ok: no floating runner label under .github/workflows"
    ;;
  *)
    echo "FAIL: grep could not scan .github/workflows (exit $status)"
    exit 1
    ;;
esac
