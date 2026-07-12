#!/usr/bin/env bash
# Dependabot file-allowlist predicate for the auto-merge flow.
#
# Reads a newline-delimited list of a pull request's changed file paths on
# stdin and prints, one per line, those a Dependabot bump is NOT expected to
# touch.  Empty output means every changed path is on the allowlist and the PR
# is safe to auto-merge; any output means a human likely edited the PR, and the
# flow should stay out of it.
#
# Allowed paths:
#   - Cargo.toml / Cargo.lock at the repo root, OR at any subdirectory depth.
#     A Cargo workspace declares a dependency in the member crate's
#     crates/<name>/Cargo.toml, which Dependabot edits directly, so a root-only
#     match strands every workspace-member bump.
#   - Any .github/workflows/*.yml (the github-actions ecosystem).
#   - The changelog file, which the auto-merge flow appends itself.
#
# Usage: printf '%s\n' "$files" | allowlist.sh [changelog-file]
set -euo pipefail

changelog_file=${1:-CHANGELOG.org}
# Escape '.' so the changelog name matches a literal dot, not any character.
changelog_re=${changelog_file//./\\.}

# --extended-regexp so the alternation and grouping read cleanly and behave
# identically under both GNU and BSD grep.  --invert-match prints the lines
# matching NONE of the allowlist patterns.  grep exits 1 when it prints nothing
# (everything allowed) — the success case here — so tolerate that under set -e.
grep --invert-match --extended-regexp \
  --regexp='^Cargo\.(toml|lock)$' \
  --regexp='/Cargo\.(toml|lock)$' \
  --regexp='^\.github/workflows/.*\.yml$' \
  --regexp="^${changelog_re}$" \
  || true
