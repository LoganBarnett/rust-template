#!/usr/bin/env bash
# test-formatters.sh — verify each formatter listed in template/treefmt.toml
# is wired up end-to-end: declared in treefmt.toml, present on PATH inside
# the spawned project's devShell, and actually transforms files matching its
# `includes` glob.
#
# For each formatter under test, the script drops a known-malformed file
# into a freshly-spawned project, runs `treefmt` from inside `nix develop`,
# and asserts the file's content changed.  Two half-failures both surface
# as failed assertions:
#
#   * formatter declared in treefmt.toml but binary missing from devShell
#     -> treefmt errors or skips, file unchanged.
#   * binary in devShell but no [formatter.X] block in treefmt.toml
#     -> treefmt never invokes it, file unchanged.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=script-common.sh
source "$SCRIPT_DIR/script-common.sh"
TMPBASE="$(mktemp --directory)"
SPAWN="$TMPBASE/format-coverage-test"
SPAWN_NAME="format-coverage-test"
CONFIG="$SCRIPT_DIR/config.json"
TEST_SUBDIR="format-coverage-test"

# Cleanup: remove the registry entry new-project.sh created in
# config.json, then remove the temp directory.  Both are best-effort —
# don't let cleanup failures mask a real test failure.
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

# ── Per-formatter malformed inputs ──────────────────────────────────────
# Each writer emits valid syntax in the target language but with style
# the corresponding formatter is guaranteed to rewrite (extra spaces,
# odd brace style, unindented bodies, etc.).  Keep these minimal —
# they're test fixtures, not example code.
write_bad_rustfmt() {
    cat > "$1" <<'EOF'
fn  main(  )  {println!("hello")  ;}
EOF
}

write_bad_alejandra() {
    cat > "$1" <<'EOF'
{ a   =   1;    b=2;c    =3; }
EOF
}

write_bad_prettier() {
    cat > "$1" <<'EOF'
h1{color:red;font-size:14px;}
EOF
}

write_bad_elm_format() {
    cat > "$1" <<'EOF'
module FormatTest exposing (main)

import Html

main  =  Html.text   "hello"
EOF
}

# Single long prose paragraph that org-fmt's reflower must wrap (the
# tool's stated scope per README: "only plain prose paragraphs are
# reflowed").  Two-space sentence terminators are present on purpose
# to exercise the upstream fix that preserves them across reflow.
write_bad_org_fmt() {
    cat > "$1" <<'EOF'
This is a single overly long line of plain prose that runs well past the eighty-column limit and must therefore be reflowed by the formatter.  The fixture also carries a second sentence so the two-space terminator survives.
EOF
}

# ── Formatter table ─────────────────────────────────────────────────────
# Parallel arrays so the elm-format hyphen does not break shell tokenizing.
FORMATTER_NAMES=(rustfmt alejandra prettier elm-format org-fmt)
FORMATTER_EXTS=(rs       nix       css      elm        org)
FORMATTER_WRITERS=(write_bad_rustfmt write_bad_alejandra write_bad_prettier write_bad_elm_format write_bad_org_fmt)

# ── Spawn the test project ──────────────────────────────────────────────
echo "Spawning test project at $SPAWN ..."
"$SCRIPT_DIR/new-project.sh" \
    --name "$SPAWN_NAME" \
    --output "$SPAWN" \
    --crates cli \
    --description "format coverage test scratch" \
    > /dev/null

# Point the spawn's foundation at this checkout before any flake evaluation.
# The emitted flake calls foundation library functions (e.g. mkMuslPackages)
# that exist on the branch under test but not on published main, so a spawn left
# at the github URL fails to evaluate `.#packages` against the older lib.
localize_foundation "$SPAWN"

# ── Assertion 1: the spawn's emitted code is already treefmt-clean ──────
# Catches regressions where a template file drifts out of formatter
# compliance, or a spawn-time expansion in crate-add.sh produces output
# that fails the formatter (the historical bug behind this assertion).
# Runs before any test fixtures are dropped, so any reported change here
# is the spawn's own emitted code — a real defect to fix in template/ or
# the generation scripts, not an artifact of this test.
echo "Asserting fresh spawn is treefmt-clean ..."
clean_log="$TMPBASE/spawn-clean.log"
if (cd "$SPAWN" && nix develop --command treefmt --ci) > "$clean_log" 2>&1; then
    echo "  PASS spawn       emitted code passes treefmt --ci"
else
    echo "  FAIL spawn       emitted code is not treefmt-clean.  Files needing format:"
    grep -E "^ERRO file has changed" "$clean_log" \
        | sed 's|.*path=|    |;s| prev_size.*||' \
        || echo "    (no ERRO per-file lines — full treefmt --ci output follows)"
    echo
    echo "─── spawn-clean.log ───"
    cat "$clean_log"
    echo "─── end ───"
    echo
    echo "Fix: format the offending source(s) under template/, or fix the"
    echo "spawn-time expansion in new-project.sh / crate-add.sh that emits"
    echo "them.  Aborting before fixture-rewrite checks."
    exit 1
fi

mkdir --parents "$SPAWN/$TEST_SUBDIR"

# ── Drop the bad files and snapshot for later comparison ───────────────
for i in "${!FORMATTER_NAMES[@]}"; do
    name="${FORMATTER_NAMES[$i]}"
    ext="${FORMATTER_EXTS[$i]}"
    writer="${FORMATTER_WRITERS[$i]}"
    file="$SPAWN/$TEST_SUBDIR/bad-${name}.${ext}"
    "$writer" "$file"
    cp "$file" "$file.original"
done

# ── Run treefmt inside the spawn's devShell ─────────────────────────────
echo "Running treefmt inside spawn devShell ..."
(
    cd "$SPAWN"
    nix develop --command treefmt 2>&1 | tail --lines=10
)

# ── Assert each formatter rewrote its target file ───────────────────────
PASS=0
FAIL=0
echo
for i in "${!FORMATTER_NAMES[@]}"; do
    name="${FORMATTER_NAMES[$i]}"
    ext="${FORMATTER_EXTS[$i]}"
    file="$SPAWN/$TEST_SUBDIR/bad-${name}.${ext}"
    if cmp --silent "$file" "$file.original"; then
        printf "  FAIL %-12s file unchanged: %s\n" "$name" "$file"
        echo "       (formatter not wired up: missing from devShell, missing"
        echo "        treefmt.toml entry, or input did not trigger a rewrite)"
        FAIL=$((FAIL + 1))
    else
        printf "  PASS %-12s file rewritten\n" "$name"
        PASS=$((PASS + 1))
    fi
done

echo
echo "Summary: $PASS passed, $FAIL failed (out of ${#FORMATTER_NAMES[@]})"
[[ $FAIL -eq 0 ]]
