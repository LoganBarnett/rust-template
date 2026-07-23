#!/usr/bin/env bash
# Stop hook: gates the end of a turn on a clean clippy run (for Rust changes)
# and a clean template-compliance review.
#
# This is not a "please remember to review" nudge — it is deterministic and
# not skippable.  Whenever the working tree holds un-reviewed code or config
# changes, the hook blocks the turn from ending until the template-compliance
# subagent has run *and reported* COMPLIANCE: PASS.  When any of those changes
# is a Rust source file, it first blocks until the workspace passes the same
# clippy gate CI enforces, so a lint failure never reaches the reviewer.
#
# The review itself is the native subagent (a Task the assistant invokes); the
# hook otherwise inspects the git working tree and the transcript in pure shell.
# The one subprocess it runs is clippy — objective and deterministic, so no
# model discretion — and never a nested `claude`.
#
# Convergence: each time the assistant addresses findings and re-runs the
# reviewer, the next Stop re-reads the verdict and releases once it is PASS.
# A bounded round cap (MAX_ROUNDS) keeps a finding the assistant genuinely
# cannot resolve from wedging the session — after the cap the gate releases
# and the unresolved findings stand in the conversation for the human.
#
# Prose-only changes (.md, .org, .txt, .rst, .adoc, LICENSE) never trigger a
# review; that work is not worth the token spend.

set -euo pipefail

# Consecutive blocks allowed within one turn before the gate gives up.  The
# first block is usually just "you have not run the reviewer yet", so this
# leaves a few rounds for actually resolving findings.
MAX_ROUNDS=4

input="$(cat)"
session_id="$(printf '%s' "$input" | jq --raw-output '.session_id // "unknown"')"
transcript_path="$(printf '%s' "$input" \
    | jq --raw-output '.transcript_path // empty')"

state_file="${TMPDIR:-/tmp}/review-stop.${session_id}.state"

# Release the gate (allow the stop) and clear any per-turn round state.
release() {
    rm --force "$state_file" 2>/dev/null || true
    exit 0
}

# Outside a git work tree, or with no transcript, there is nothing to gate.
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    release
fi
if [[ -z "$transcript_path" || ! -f "$transcript_path" ]]; then
    release
fi

# 1. Are there un-reviewed code/config changes in the working tree?  Reading
#    git (rather than reconstructing edits from the transcript) catches edits
#    made through Bash/sed/heredoc, not just Edit/Write/MultiEdit.
qualifying=""
# Set when any qualifying change is a Rust source file, which arms the
# deterministic clippy gate below.
has_rust=""
# The working-tree file list normally comes from git.  Tests set
# REVIEW_STOP_GIT_FILES_FILE to a fixture holding a newline-separated list, so
# the gate's behaviour can be exercised without mutating a real working tree.
if [[ -n "${REVIEW_STOP_GIT_FILES_FILE:-}" ]]; then
    changed_files="$(cat "$REVIEW_STOP_GIT_FILES_FILE")"
else
    changed_files="$(
        {
            git -c core.quotepath=false diff --name-only
            git -c core.quotepath=false diff --cached --name-only
            git -c core.quotepath=false ls-files --others --exclude-standard
        } 2>/dev/null | sort --unique
    )"
fi
while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    lower="$(printf '%s' "$f" | tr '[:upper:]' '[:lower:]')"
    case "$lower" in
        *.md|*.org|*.txt|*.rst|*.adoc) ;;
        license|*/license|*.license) ;;
        *) qualifying+="$f"$'\n'
           case "$lower" in *.rs) has_rust=1 ;; esac ;;
    esac
done < <(printf '%s\n' "$changed_files")

if [[ -z "$qualifying" ]]; then
    release
fi

# 1b. Deterministic clippy gate for Rust changes.  Unlike the compliance
#     review below — a subagent's judgment — this is objective: when a change
#     touches Rust, the workspace must pass the same clippy gate CI enforces
#     before the turn may end, shifting a lint failure off the reviewer and the
#     pipeline.  Tests inject REVIEW_STOP_CLIPPY_CMD to drive the branch without
#     a real compile; real runs use the devshell toolchain (cargo directly when
#     on PATH, else `nix develop`).  When no toolchain is reachable the gate
#     steps aside rather than wedge a machine that cannot run it.
if [[ -n "$has_rust" ]]; then
    if [[ -n "${REVIEW_STOP_CLIPPY_CMD:-}" ]]; then
        clippy_cmd="$REVIEW_STOP_CLIPPY_CMD"
    elif command -v cargo >/dev/null 2>&1; then
        clippy_cmd="cargo clippy --workspace --all-targets --all-features \
-- --deny warnings"
    elif command -v nix >/dev/null 2>&1; then
        clippy_cmd="nix develop --command cargo clippy --workspace \
--all-targets --all-features -- --deny warnings"
    else
        clippy_cmd=""
    fi

    if [[ -n "$clippy_cmd" ]]; then
        clippy_state="${TMPDIR:-/tmp}/review-stop-clippy.${session_id}.state"
        if clippy_out="$(bash -c "$clippy_cmd" 2>&1)"; then
            # Clean: clear the clippy round counter and fall through to the
            # compliance review gate.
            rm --force "$clippy_state" 2>/dev/null || true
        else
            # Bound consecutive clippy blocks so a warning the assistant
            # genuinely cannot clear (e.g. pre-existing in a drifted repo)
            # does not wedge the session — same escape valve MAX_ROUNDS gives
            # the review gate.
            prev=0
            if [[ -f "$clippy_state" ]]; then
                read -r prev < "$clippy_state" || true
            fi
            [[ "$prev" =~ ^[0-9]+$ ]] || prev=0
            cur=$((prev + 1))
            if (( cur > MAX_ROUNDS )); then
                printf 'review-stop: releasing after %d unresolved clippy rounds\n' \
                    "$MAX_ROUNDS" >&2
                rm --force "$clippy_state" 2>/dev/null || true
            else
                printf '%s\n' "$cur" > "$clippy_state"
                reason="clippy reported problems on the Rust changes this turn. \
Resolve every warning before ending the turn — this is the same gate CI \
enforces:

    cargo clippy --workspace --all-targets --all-features -- --deny warnings

${clippy_out}"
                jq --null-input --arg reason "$reason" \
                    '{decision: "block", reason: $reason}'
                exit 0
            fi
        fi
    fi
fi

# 2. Find the line index of the last real user prompt, so "this turn" is well
#    defined.  A "user" entry whose content is a tool_result is a tool
#    response, not a prompt; we want the last text prompt.
last_prompt_idx="$(jq --slurp --raw-input '
    split("\n")
    | map(select(length > 0))
    | map(fromjson? // empty)
    | to_entries
    | map(select(
        .value.type == "user"
        and (
            ((.value.message.content | type) == "string")
            or (
                ((.value.message.content | type) == "array")
                and (.value.message.content | any(.type == "text"))
            )
        )
      ))
    | (last // {key: -1}).key
' "$transcript_path")"
last_prompt_idx="${last_prompt_idx:--1}"

# 3. Read the verdict of the most recent template-compliance review since that
#    prompt.  Each review Task is matched by id to its tool_result, and the
#    machine-readable COMPLIANCE: line the subagent emits is read from it.
review="$(jq --slurp --raw-input --argjson skip "$last_prompt_idx" '
    ( split("\n") | map(select(length > 0)) | map(fromjson? // empty)
      | .[($skip + 1):] ) as $all
    | ([ $all[]
         | select(.type == "assistant")
         | .message.content[]?
         # The subagent-spawning tool is named "Task" in stock Claude Code but
         # "Agent" in some harnesses; match either so the review is detected
         # regardless of which one recorded the call.
         | select(.type == "tool_use"
                  and (.name == "Task" or .name == "Agent")
                  and (.input.subagent_type == "template-compliance"))
         | .id ]) as $ids
    | ([ $all[]
         | select(.type == "user")
         | .message.content[]?
         | select(.type == "tool_result")
         | . as $r
         | ($r.tool_use_id) as $tid
         | select(($ids | index($tid)) != null)
         | ($r.content
            | if type == "array" then (map(.text? // "") | join("\n"))
              elif type == "string" then .
              else "" end) ]) as $results
    | if ($results | length) == 0 then {verdict: "none", text: ""}
      elif ($results[-1] | test("COMPLIANCE:\\s*PASS")) then
        {verdict: "pass", text: $results[-1]}
      else {verdict: "findings", text: $results[-1]} end
' "$transcript_path")"

verdict="$(printf '%s' "$review" | jq --raw-output '.verdict')"

# 4. A clean review releases the gate immediately.
if [[ "$verdict" == "pass" ]]; then
    release
fi

# 5. Otherwise block — but bound the consecutive blocks per turn so a finding
#    the assistant cannot resolve does not wedge the session.  The round count
#    is keyed to the last prompt index, so each turn starts with a fresh budget.
prev_idx=""
prev_count=0
if [[ -f "$state_file" ]]; then
    read -r prev_idx prev_count < "$state_file" || true
fi
[[ "$prev_idx" == "$last_prompt_idx" ]] || prev_count=0
[[ "$prev_count" =~ ^[0-9]+$ ]] || prev_count=0
count=$((prev_count + 1))

if (( count > MAX_ROUNDS )); then
    # Give up gracefully: release so the turn can end.  The unresolved
    # findings remain visible in the conversation for the human to judge.
    printf 'template-compliance: releasing after %d unresolved rounds\n' \
        "$MAX_ROUNDS" >&2
    release
fi

printf '%s %s\n' "$last_prompt_idx" "$count" > "$state_file"

if [[ "$verdict" == "findings" ]]; then
    findings_text="$(printf '%s' "$review" | jq --raw-output '.text')"
    reason="The template-compliance review reported findings that are not yet \
resolved:

${findings_text}

Address every finding, then re-run the template-compliance subagent.  This \
gate releases only when the review reports COMPLIANCE: PASS."
else
    reason="Code or config files changed this turn, but the \
template-compliance review has not run.  Invoke the template-compliance \
subagent (Task tool, subagent_type=\"template-compliance\"), then resolve \
every finding it reports.  This gate releases only when the review reports \
COMPLIANCE: PASS."
fi

jq --null-input --arg reason "$reason" '{decision: "block", reason: $reason}'
