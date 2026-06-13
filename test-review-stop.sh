#!/usr/bin/env bash
# Unit test for the code-review Stop hook
# (template/.claude/hooks/review-stop.sh).
#
# The hook is pure shell that inspects the git working tree and the session
# transcript, then either releases the turn or emits a block decision.  Both
# inputs are injectable: the transcript path arrives in the stdin JSON, and
# the working-tree file list is faked with REVIEW_STOP_GIT_FILES_FILE.  That
# lets this test drive every branch deterministically without a real working
# tree or a live Claude session.
#
# Each case feeds a crafted transcript and file list, then asserts whether the
# hook releases (no stdout) or blocks (a {"decision":"block"} JSON document).
# The load-bearing case is "Agent" tool detection: a harness that records the
# review subagent under the name "Agent" rather than "Task" must still satisfy
# the gate.  A regression there silently wedged a real session.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOOK="$SCRIPT_DIR/.claude/hooks/review-stop.sh"

# The hook probes the working tree with `git rev-parse --is-inside-work-tree`;
# run from the repo root so that probe succeeds the same way it does in a real
# session.
cd "$SCRIPT_DIR"

# These coreutils flags have no portable long form on BSD/macOS, so the short
# spellings are deliberate: mktemp -d (make a directory) and rm -rf (recursive,
# force) for the cleanup trap.
TMPBASE="$(mktemp -d)"
trap 'rm -rf "$TMPBASE"' EXIT

PASS=0
FAIL=0

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

# Invoke the hook with a synthetic session.  Populates HOOK_OUT (stdout) for
# the assertions below.  The hook always exits 0, signalling block-versus-
# release through stdout content rather than the exit code: a release prints
# nothing, a block prints a decision document.  State files land under TMPBASE
# so the MAX_ROUNDS accounting stays isolated from any real /tmp state.
HOOK_OUT=""
run_hook() {
    local session="$1" transcript="$2" gitfiles="$3"
    set +e
    HOOK_OUT="$(
        printf '{"session_id":"%s","transcript_path":"%s"}' \
            "$session" "$transcript" \
            | TMPDIR="$TMPBASE" REVIEW_STOP_GIT_FILES_FILE="$gitfiles" \
                bash "$HOOK" 2>/dev/null
    )"
    set -e
}

assert_releases() {
    if printf '%s' "$HOOK_OUT" | grep --quiet '"decision"'; then
        echo "  assertion failed: expected release, got block: $HOOK_OUT" >&2
        return 1
    fi
}

# Assert the hook blocked, and (optionally) that the block reason contains the
# given substring.
assert_blocks() {
    if ! printf '%s' "$HOOK_OUT" | grep --quiet '"block"'; then
        echo "  assertion failed: expected block, got: ${HOOK_OUT:-<empty>}" >&2
        return 1
    fi
    if [[ -n "${1:-}" ]] \
        && ! printf '%s' "$HOOK_OUT" | grep --quiet -- "$1"; then
        echo "  assertion failed: block reason missing '$1': $HOOK_OUT" >&2
        return 1
    fi
}

# ── Fixtures ─────────────────────────────────────────────────────────

# Working-tree file lists.  The hook treats prose extensions as not worth a
# review; code/config files are gated.
files_prose="$TMPBASE/files-prose"
printf '%s\n' "README.org" "docs/notes.txt" > "$files_prose"

files_code="$TMPBASE/files-code"
printf '%s\n' "src/main.rs" > "$files_code"

files_none="$TMPBASE/files-none"
: > "$files_none"

# A transcript with a single user prompt and no review.
tx_noreview="$TMPBASE/tx-noreview.jsonl"
cat > "$tx_noreview" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
EOF

# A passing review recorded under the "Agent" tool name (the regression case).
tx_agent_pass="$TMPBASE/tx-agent-pass.jsonl"
cat > "$tx_agent_pass" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
EOF

# The same passing review recorded under the stock "Task" tool name.
tx_task_pass="$TMPBASE/tx-task-pass.jsonl"
cat > "$tx_task_pass" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Task","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
EOF

# A review that reported findings rather than passing.
tx_findings="$TMPBASE/tx-findings.jsonl"
cat > "$tx_findings" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: FINDINGS\nsomething is off"}]}]}}
EOF

# A passing review that predates a newer user prompt, so it must not count
# toward the current turn.
tx_stale_review="$TMPBASE/tx-stale-review.jsonl"
cat > "$tx_stale_review" <<'EOF'
{"type":"user","message":{"content":"first prompt"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
{"type":"user","message":{"content":"second prompt with fresh changes"}}
EOF

# ── Cases ────────────────────────────────────────────────────────────

# Prose-only changes never trigger a review.
test_prose_only_releases() {
    run_hook "prose" "$tx_noreview" "$files_prose"
    assert_releases
}

# No working-tree changes at all: nothing to gate.
test_no_changes_releases() {
    run_hook "none" "$tx_noreview" "$files_none"
    assert_releases
}

# Code changed and no review ran: block with the "has not run" reason.
test_code_without_review_blocks() {
    run_hook "code-noreview" "$tx_noreview" "$files_code"
    assert_blocks "has not run"
}

# Regression: a passing review recorded as "Agent" releases the gate.
test_agent_review_pass_releases() {
    run_hook "agent-pass" "$tx_agent_pass" "$files_code"
    assert_releases
}

# Back-compat: a passing review recorded as "Task" still releases.
test_task_review_pass_releases() {
    run_hook "task-pass" "$tx_task_pass" "$files_code"
    assert_releases
}

# A review reporting findings keeps the gate closed.
test_findings_blocks() {
    run_hook "findings" "$tx_findings" "$files_code"
    assert_blocks "reported findings"
}

# A passing review from before the latest prompt does not satisfy this turn.
test_stale_review_blocks() {
    run_hook "stale" "$tx_stale_review" "$files_code"
    assert_blocks "has not run"
}

# A missing transcript leaves nothing to gate on.
test_missing_transcript_releases() {
    run_hook "missing" "$TMPBASE/does-not-exist.jsonl" "$files_code"
    assert_releases
}

# MAX_ROUNDS safety valve: after the bounded number of consecutive blocks
# within one turn, the gate releases so a finding the assistant genuinely
# cannot resolve does not wedge the session.  The hook caps at four blocks, so
# the fifth invocation of an otherwise-blocking turn releases.
test_max_rounds_releases() {
    local i
    for i in 1 2 3 4; do
        run_hook "rounds" "$tx_noreview" "$files_code"
        assert_blocks "has not run" || return 1
    done
    run_hook "rounds" "$tx_noreview" "$files_code"
    assert_releases
}

run_test "prose-only-releases" test_prose_only_releases
run_test "no-changes-releases" test_no_changes_releases
run_test "code-without-review-blocks" test_code_without_review_blocks
run_test "agent-review-pass-releases" test_agent_review_pass_releases
run_test "task-review-pass-releases" test_task_review_pass_releases
run_test "findings-blocks" test_findings_blocks
run_test "stale-review-blocks" test_stale_review_blocks
run_test "missing-transcript-releases" test_missing_transcript_releases
run_test "max-rounds-releases" test_max_rounds_releases

echo ""
echo "$PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
