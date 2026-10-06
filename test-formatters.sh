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
#
# The script then commits through the spawn's pre-commit hook.  It asserts
# that the hook formats and commits only staged content.

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

# ── Pre-commit hook: only staged content is formatted and committed ─────
HOOK=".cargo-husky/hooks/pre-commit"
MAIN_RS="crates/cli/src/main.rs"
LIB_RS="crates/lib/src/lib.rs"
head_main="$TMPBASE/head-main.rs"
working_main="$TMPBASE/working-main.rs"
commit_log="$TMPBASE/hook-commit.log"

# Records one hook assertion.  The first argument is the label.  The rest is
# the command whose exit status decides the result.
hook_check() {
    local label="$1"
    shift
    if "$@"; then
        printf "  PASS %-12s %s\n" "pre-commit" "$label"
        PASS=$((PASS + 1))
    else
        printf "  FAIL %-12s %s\n" "pre-commit" "$label"
        FAIL=$((FAIL + 1))
    fi
}

hook_commit() {
    if (cd "$SPAWN" && nix develop --command \
            git commit --quiet --message "hook probe") > "$commit_log" 2>&1; then
        return 0
    fi
    cat "$commit_log"
    return 1
}

head_lacks_unstaged_hunk() {
    ! grep --quiet --fixed-strings -- 'unstaged_probe' "$head_main"
}

# The two-space indent comes from rustfmt.toml.  It shows the formatter found
# the project's configuration and not its own defaults.
head_holds_formatted_staged_hunk() {
    grep --quiet --fixed-strings --line-regexp -- \
        'fn staged_probe() {' "$head_main" \
        && grep --quiet --fixed-strings --line-regexp -- \
            '  let _x = 1;' "$head_main"
}

fully_staged_file_is_synced() {
    grep --quiet --fixed-strings --line-regexp -- \
        'fn synced_probe() {' "$SPAWN/$LIB_RS" \
        && [[ -z "$(cd "$SPAWN" && git status --porcelain -- "$LIB_RS")" ]]
}

echo
echo "Committing through the spawn's pre-commit hook ..."
(
    cd "$SPAWN"
    git init --quiet --initial-branch main
    git config user.name "Hook Test"
    git config user.email "hook-test@example.invalid"
    git config commit.gpgsign false
    # A machine-wide core.hooksPath would otherwise decide whether the hook
    # under test runs at all.
    git config core.hooksPath "$(dirname "$HOOK")"
    git add --all
    git commit --quiet --no-verify --message "baseline"

    # main.rs declares an out-of-line module.  The formatter must resolve it
    # from the staged tree for the commit to succeed.
    printf 'fn  staged_probe(  )  {let _x=1;}\n' >> "$MAIN_RS"
    printf 'fn  synced_probe(  )  {let _z=3;}\n' >> "$LIB_RS"
    git add "$MAIN_RS" "$LIB_RS"
    # Appending after the add leaves this function out of the index.  That is
    # the state `git add --patch` produces when one hunk is declined.
    printf 'fn  unstaged_probe(  )  {let _y=2;}\n' >> "$MAIN_RS"
)
cp "$SPAWN/$MAIN_RS" "$working_main"

# The spawn runs the template's copy of the hook.  Equality extends every
# result below to this repo's own copy.
hook_check "this repo's hook matches the template's" \
    cmp --silent "$SCRIPT_DIR/$HOOK" "$SCRIPT_DIR/template/$HOOK"
hook_check "commit through the hook succeeds" hook_commit
(cd "$SPAWN" && git show "HEAD:$MAIN_RS") > "$head_main"
hook_check "unstaged hunk stays out of the commit" head_lacks_unstaged_hunk
hook_check "staged hunk is committed formatted" \
    head_holds_formatted_staged_hunk
hook_check "working file keeps its unstaged hunk untouched" \
    cmp --silent "$SPAWN/$MAIN_RS" "$working_main"
hook_check "fully staged file is formatted in the working tree too" \
    fully_staged_file_is_synced

# format-staged carries its own treefmt.  This commit runs with every PATH
# directory that holds a treefmt removed.  The formatters stay, since they
# live in other directories.  The commit fails unless the hook finds the
# treefmt the wrapper carries.
hook_commit_without_treefmt() {
    if (cd "$SPAWN" && nix develop --command bash -c '
            set -euo pipefail
            IFS=: read -ra dirs <<< "$PATH"
            kept=()
            for dir in "${dirs[@]}"; do
                [[ -x "$dir/treefmt" ]] || kept+=("$dir")
            done
            PATH=$(IFS=:; printf "%s" "${kept[*]}")
            export PATH
            if command -v treefmt > /dev/null; then
                echo "treefmt is still on PATH, so this proves nothing" >&2
                exit 1
            fi
            git commit --quiet --message "hook probe without treefmt"
        ') > "$commit_log" 2>&1; then
        return 0
    fi
    cat "$commit_log"
    return 1
}

head_holds_formatted_bundled_probe() {
    (cd "$SPAWN" && git show "HEAD:$LIB_RS") \
        | grep --quiet --fixed-strings --line-regexp -- 'fn bundled_probe() {'
}

(
    cd "$SPAWN"
    printf 'fn  bundled_probe(  )  {let _w=4;}\n' >> "$LIB_RS"
    git add "$LIB_RS"
)
hook_check "commit succeeds with no treefmt on the caller's PATH" \
    hook_commit_without_treefmt
hook_check "that commit holds the staged hunk formatted" \
    head_holds_formatted_bundled_probe

echo
echo "Summary: $PASS passed, $FAIL failed (out of $((PASS + FAIL)))"
[[ $FAIL -eq 0 ]]
