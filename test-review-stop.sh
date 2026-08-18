#!/usr/bin/env bash
# Unit test for the code-review Stop hook
# (template/.claude/hooks/review-stop.sh).
#
# The hook inspects the git working tree and the session transcript, runs a
# clippy gate when Rust files changed, then either releases the turn or emits a
# block decision.  Every input is injectable: the transcript path and the
# permission mode arrive in the stdin JSON, the working-tree file list is faked
# with REVIEW_STOP_GIT_FILES_FILE, and the clippy command is faked with
# REVIEW_STOP_CLIPPY_CMD.  That lets this test drive every branch
# deterministically without a real working tree, a real compile, or a live
# Claude session.  The hook keeps its per-session state under TMPDIR, which is
# pointed at a scratch directory here, so cases that span several Stops (the
# round caps, the unchanged-tree release) share state only with themselves.
#
# Each case feeds a crafted transcript and file list, then asserts whether the
# hook releases (no stdout) or blocks (a {"decision":"block"} JSON document).
# The load-bearing case is "Agent" tool detection: a harness that records the
# review subagent under the name "Agent" rather than "Task" must still satisfy
# the gate.  A regression there silently wedged a real session.
#
# Usage: test-review-stop.sh [--target PATH]
#
# By default the suite exercises this repo's own copy of the hook.  --target
# points it at another copy — how the compliance checker
# (crates/compliance-lib, kind template-suite-passes) runs the template's
# current suite against a spawn's hook, so a spawn whose hook has gone stale
# is flagged by behaviour rather than by its source text.  Whichever hook is
# under test, the suite runs from this repo's root: the hook probes for a git
# work tree before doing anything, and the fixture seams above replace every
# git read it would otherwise make, so the target's own working tree is never
# consulted and the outcome does not depend on it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOOK="$SCRIPT_DIR/.claude/hooks/review-stop.sh"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)
            [[ $# -ge 2 ]] || { echo "--target requires a path" >&2; exit 2; }
            HOOK="$2"
            shift 2
            ;;
        *)
            echo "usage: $0 [--target PATH]" >&2
            exit 2
            ;;
    esac
done
[[ -f "$HOOK" ]] || { echo "no hook at $HOOK" >&2; exit 2; }
# Resolve the target before changing directory, so a relative --target given
# from the caller's directory still points where the caller meant.
HOOK="$(cd "$(dirname "$HOOK")" && pwd)/$(basename "$HOOK")"

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
    # The clippy gate command is injected so tests never run a real compile.
    # It defaults to a no-op pass; cases that exercise the clippy gate override
    # it with a failing command.  Without this, the existing .rs fixtures would
    # shell out to real clippy on every run.
    local clippy_cmd="${4:-true}"
    # The permission mode is sent only when a case names one.  Left empty, the
    # field is omitted altogether, which is what a harness that predates it
    # sends — so every case that does not care models that harness.
    local permission_mode="${5:-}"
    local stdin_json
    stdin_json="$(jq --null-input --compact-output \
        --arg session "$session" --arg transcript "$transcript" \
        --arg mode "$permission_mode" \
        '{session_id: $session, transcript_path: $transcript}
         + (if $mode == "" then {} else {permission_mode: $mode} end)')"
    set +e
    HOOK_OUT="$(
        printf '%s' "$stdin_json" \
            | TMPDIR="$TMPBASE" REVIEW_STOP_GIT_FILES_FILE="$gitfiles" \
                REVIEW_STOP_CLIPPY_CMD="$clippy_cmd" \
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

# The same change plus one more file: a different working tree from files_code,
# so the fingerprint the gate stores on release no longer matches.
files_code_two="$TMPBASE/files-code-two"
printf '%s\n' "src/main.rs" "src/lib.rs" > "$files_code_two"

# A code/config change that is not Rust: gated for review, but the clippy gate
# must leave it alone.
files_code_nonrust="$TMPBASE/files-code-nonrust"
printf '%s\n' "flake.nix" > "$files_code_nonrust"

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

# A background subagent: the tool_result for the spawn holds only launch
# metadata, and the verdict arrives later as a completion notification.  Two
# details are taken from a real transcript rather than guessed, because both
# defeat a naive reader: the notification is a tool_result whose text sits in
# .content rather than .text, and it is attached to whichever unrelated tool
# call happened to be in flight, so its tool_use_id does not identify the
# review.  The launch metadata deliberately carries no COMPLIANCE line, so a
# gate that read it as the report would mistake a launch for a verdict.
tx_background_pass="$TMPBASE/tx-background-pass.jsonl"
cat > "$tx_background_pass" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","id":"wait1","input":{"command":"sleep"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"wait1","content":"waited\n\n<task-notification>\n<status>completed</status>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}]}}
EOF

# The same delivery path, but the background review found something.
tx_background_findings="$TMPBASE/tx-background-findings.jsonl"
cat > "$tx_background_findings" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","id":"wait1","input":{"command":"sleep"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"wait1","content":"waited\n\n<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>"}]}}
EOF

# The notification delivered as a text block rather than a tool_result, which is
# how it reads when no tool call is in flight to carry it.
tx_background_pass_text="$TMPBASE/tx-background-pass-text.jsonl"
cat > "$tx_background_pass_text" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"user","message":{"content":[{"type":"text","text":"<task-notification>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}]}}
EOF

# The third and, in practice, most common delivery: the notification text lands
# in a `toolUseResult` field beside `.message` rather than inside the content
# array at all.  Taken from a real transcript — a reader that walks the content
# blocks finds nothing here, which is precisely how this gate came to spin.
tx_background_pass_sibling="$TMPBASE/tx-background-pass-sibling.jsonl"
cat > "$tx_background_pass_sibling" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","id":"wait1","input":{"command":"sleep"}}]}}
{"type":"user","toolUseResult":{"stdout":"waited\n\n<task-notification>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"},"message":{"content":[{"type":"tool_result","tool_use_id":"wait1","content":"waited"}]}}
EOF

# The fourth delivery, and the one a session sees whenever no tool call is in
# flight when the review finishes: the notification is queued, and the
# transcript records the queue bookkeeping (queue-operation enqueue/remove)
# and the queued_command attachment that hands it to the model — none of them
# a user turn.  Taken from a real transcript, where a reader admitting only
# user turns never saw the verdict and blocked on a passing review.
tx_queued_pass="$TMPBASE/tx-queued-pass.jsonl"
cat > "$tx_queued_pass" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<status>completed</status>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}
{"type":"queue-operation","operation":"remove","content":"<task-notification>\n<status>completed</status>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}
{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<status>completed</status>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}}
EOF

# The same queued delivery carrying findings, then a later queued PASS after
# they were addressed: the most recent verdict is the one that counts, in this
# shape as in the others.
tx_queued_findings_then_pass="$TMPBASE/tx-queued-findings-then-pass.jsonl"
cat > "$tx_queued_findings_then_pass" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev2","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev2","content":"Async agent launched successfully.\nagentId: def456"}]}}
{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}}
EOF

# Queued findings with nothing after them keep the gate closed.
tx_queued_findings="$TMPBASE/tx-queued-findings.jsonl"
cat > "$tx_queued_findings" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>"}
{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>"}}
EOF

# The assistant discussing this gate is not a verdict.  Its prose quotes
# "COMPLIANCE: PASS" whenever it explains the rule, and a notification tag when
# it explains the delivery — so reading assistant turns back would let the
# assistant release the gate by talking about it.
tx_assistant_prose="$TMPBASE/tx-assistant-prose.jsonl"
cat > "$tx_assistant_prose" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"The gate wants a <task-notification> carrying COMPLIANCE: PASS before it will release."}]}}
EOF

# This gate's own block reason, re-injected as a user turn after a passing
# review.  It must not read as a fresh prompt (which would hide the review) and
# must not be mistaken for a verdict, even though it quotes the words
# "COMPLIANCE: PASS".
tx_hook_reinjection="$TMPBASE/tx-hook-reinjection.jsonl"
cat > "$tx_hook_reinjection" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
{"type":"user","message":{"content":[{"type":"text","text":"Code or config files changed this turn, but the template-compliance review has not run.  This gate releases only when the review reports COMPLIANCE: PASS."}]}}
EOF

# A findings block quotes the report it is complaining about, so when it comes
# back as a user turn it carries both a task-notification tag and — from the
# gate's own closing sentence — the words "COMPLIANCE: PASS".  Nothing behind it
# passed, so the gate must still block: this is the shape that would release on
# the strength of the hook's own prose if block text were admitted as a verdict.
tx_hook_reinjection_only="$TMPBASE/tx-hook-reinjection-only.jsonl"
cat > "$tx_hook_reinjection_only" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"Async agent launched successfully.\nagentId: abc123"}]}]}}
{"type":"user","message":{"content":[{"type":"text","text":"The template-compliance review reported findings that are not yet resolved:\n\n<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>\n\nAddress every finding, then re-run the template-compliance subagent.  This gate releases only when the review reports COMPLIANCE: PASS."}]}}
EOF

# The clippy block reason takes the same round trip as the compliance one, and
# both filters carry a separate marker for it.  A passing review sits behind it,
# so the gate releases — unless the clippy marker is dropped, in which case the
# re-injected block reads as a fresh prompt and hides that review.
tx_clippy_reinjection="$TMPBASE/tx-clippy-reinjection.jsonl"
cat > "$tx_clippy_reinjection" <<'EOF'
{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
{"type":"user","message":{"content":[{"type":"text","text":"clippy reported problems on the Rust changes this turn. Resolve every warning before ending the turn."}]}}
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

# A background review whose verdict arrives as a task-notification, rather than
# in the tool_result, still releases the gate.  Without this the launch
# metadata is the only thing the gate can see, and it never converges.
test_background_review_pass_releases() {
    run_hook "background-pass" "$tx_background_pass" "$files_code"
    assert_releases
}

# The same delivery path must carry findings through as well.
test_background_review_findings_blocks() {
    run_hook "background-findings" "$tx_background_findings" "$files_code"
    assert_blocks "reported findings"
}

# The notification also releases the gate when it arrives as a text block.
test_background_review_text_notification_releases() {
    run_hook "background-pass-text" "$tx_background_pass_text" "$files_code"
    assert_releases
}

# The notification also releases the gate when it arrives in a toolUseResult
# field beside the message.
test_background_review_sibling_notification_releases() {
    run_hook "background-pass-sibling" "$tx_background_pass_sibling" \
        "$files_code"
    assert_releases
}

# A verdict recorded only as queue bookkeeping and a queued_command attachment
# — no user turn carries it — still releases the gate.
test_queued_notification_releases() {
    run_hook "queued-pass" "$tx_queued_pass" "$files_code"
    assert_releases
}

# Queued findings followed by a queued pass release: the latest verdict wins.
test_queued_findings_then_pass_releases() {
    run_hook "queued-findings-then-pass" "$tx_queued_findings_then_pass" \
        "$files_code"
    assert_releases
}

# Queued findings with no later pass keep the gate closed.
test_queued_findings_blocks() {
    run_hook "queued-findings" "$tx_queued_findings" "$files_code"
    assert_blocks "reported findings"
}

# The assistant talking about the gate never satisfies it.
test_assistant_prose_is_not_a_verdict() {
    run_hook "assistant-prose" "$tx_assistant_prose" "$files_code"
    assert_blocks "has not run"
}

# The gate's own block reason, re-injected as a user turn, is not a new prompt:
# the passing review before it still counts, so the gate releases instead of
# spinning.
test_hook_reinjection_keeps_the_turn() {
    run_hook "reinjection" "$tx_hook_reinjection" "$files_code"
    assert_releases
}

# A re-injected findings block is never itself a verdict, even though it carries
# a task-notification tag and quotes "COMPLIANCE: PASS" while explaining the
# rule.  Only the launch metadata sits behind it, so the gate stays closed.
test_hook_reinjection_is_not_a_verdict() {
    run_hook "reinjection-only" "$tx_hook_reinjection_only" "$files_code"
    assert_blocks "has not run"
}

# The clippy block reason gets the same treatment as the compliance one: it is
# not a fresh prompt, so the passing review before it still counts.
test_clippy_reinjection_keeps_the_turn() {
    run_hook "clippy-reinjection" "$tx_clippy_reinjection" "$files_code_nonrust"
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

# Plan mode is read-only, so the gate stands aside: code changes sit in the
# tree and no review has run, yet the turn ends.  This is also the release
# valve for stepping out of a gated task into a discussion.
test_plan_mode_releases() {
    run_hook "plan" "$tx_noreview" "$files_code" true plan
    assert_releases
}

# Only plan mode is read-only.  Any other named mode is gated as before, so a
# release here would mean the hook keys on the field being present rather than
# on its value.
test_non_plan_mode_still_blocks() {
    run_hook "not-plan" "$tx_noreview" "$files_code" true default
    assert_blocks "has not run"
}

# A passing review releases and records what it released on.  A later Stop in
# the same session — a new prompt with no review since — that finds the same
# working tree has nothing new to review, so it releases rather than demanding
# the review again.  Without this every discussion turn after reviewed but
# uncommitted edits re-blocks.
test_unchanged_tree_releases_after_pass() {
    run_hook "unchanged" "$tx_agent_pass" "$files_code"
    assert_releases || return 1
    run_hook "unchanged" "$tx_stale_review" "$files_code"
    assert_releases
}

# The same session, but the tree has moved on since the release: the recorded
# fingerprint no longer matches, so the gate re-arms and demands a review.
test_changed_tree_rearms_gate() {
    run_hook "unchanged" "$tx_stale_review" "$files_code_two"
    assert_blocks "has not run"
}

# MAX_ROUNDS safety valve: after the bounded number of consecutive blocks
# within one turn, the gate releases so a finding the assistant genuinely
# cannot resolve does not wedge the session.  The hook caps at four blocks, so
# the fifth invocation of an otherwise-blocking turn releases.  Giving up also
# records the tree, so a sixth Stop on the same content releases at once rather
# than opening another four-block cycle.
test_max_rounds_releases() {
    local i
    for i in 1 2 3 4; do
        run_hook "rounds" "$tx_noreview" "$files_code"
        assert_blocks "has not run" || return 1
    done
    run_hook "rounds" "$tx_noreview" "$files_code"
    assert_releases || return 1
    run_hook "rounds" "$tx_noreview" "$files_code"
    assert_releases
}

# A Rust change with a failing clippy gate blocks before the review is even
# consulted — the transcript here carries a passing review, so a release would
# prove clippy did not gate.
test_clippy_failure_blocks() {
    run_hook "clippy-fail" "$tx_agent_pass" "$files_code" 'echo clippy-boom; exit 1'
    assert_blocks "clippy reported problems"
}

# The clippy gate only fires for Rust files.  A non-Rust code change with a
# failing clippy command must skip clippy and fall through to the review gate,
# blocking with the review reason rather than the clippy one.
test_clippy_skipped_for_non_rust() {
    run_hook "clippy-skip" "$tx_noreview" "$files_code_nonrust" \
        'echo clippy-boom; exit 1'
    assert_blocks "has not run"
}

# clippy safety valve: after MAX_ROUNDS consecutive failing clippy runs the
# gate releases (then falls through to the review, which passes here), so an
# unfixable warning cannot wedge the session.
test_clippy_max_rounds_releases() {
    local i
    for i in 1 2 3 4; do
        run_hook "clippy-rounds" "$tx_agent_pass" "$files_code" 'exit 1'
        assert_blocks "clippy reported problems" || return 1
    done
    run_hook "clippy-rounds" "$tx_agent_pass" "$files_code" 'exit 1'
    assert_releases
}

run_test "prose-only-releases" test_prose_only_releases
run_test "no-changes-releases" test_no_changes_releases
run_test "code-without-review-blocks" test_code_without_review_blocks
run_test "agent-review-pass-releases" test_agent_review_pass_releases
run_test "task-review-pass-releases" test_task_review_pass_releases
run_test "background-review-pass-releases" test_background_review_pass_releases
run_test "background-review-findings-blocks" \
    test_background_review_findings_blocks
run_test "background-review-text-notification-releases" \
    test_background_review_text_notification_releases
run_test "background-review-sibling-notification-releases" \
    test_background_review_sibling_notification_releases
run_test "queued-notification-releases" test_queued_notification_releases
run_test "queued-findings-then-pass-releases" \
    test_queued_findings_then_pass_releases
run_test "queued-findings-blocks" test_queued_findings_blocks
run_test "assistant-prose-is-not-a-verdict" \
    test_assistant_prose_is_not_a_verdict
run_test "hook-reinjection-keeps-the-turn" test_hook_reinjection_keeps_the_turn
run_test "hook-reinjection-is-not-a-verdict" \
    test_hook_reinjection_is_not_a_verdict
run_test "clippy-reinjection-keeps-the-turn" \
    test_clippy_reinjection_keeps_the_turn
run_test "findings-blocks" test_findings_blocks
run_test "stale-review-blocks" test_stale_review_blocks
run_test "missing-transcript-releases" test_missing_transcript_releases
run_test "plan-mode-releases" test_plan_mode_releases
run_test "non-plan-mode-still-blocks" test_non_plan_mode_still_blocks
run_test "unchanged-tree-releases-after-pass" \
    test_unchanged_tree_releases_after_pass
run_test "changed-tree-rearms-gate" test_changed_tree_rearms_gate
run_test "max-rounds-releases" test_max_rounds_releases
run_test "clippy-failure-blocks" test_clippy_failure_blocks
run_test "clippy-skipped-for-non-rust" test_clippy_skipped_for_non_rust
run_test "clippy-max-rounds-releases" test_clippy_max_rounds_releases

echo ""
echo "$PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
