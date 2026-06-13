#!/usr/bin/env bash
# Stop hook: gates the end of a turn on a clean template-compliance review.
#
# This is not a "please remember to review" nudge — it is deterministic and
# not skippable.  Whenever the working tree holds un-reviewed code or config
# changes, the hook blocks the turn from ending until the template-compliance
# subagent has run *and reported* COMPLIANCE: PASS.
#
# The review itself is the native subagent (a Task the assistant invokes);
# this hook only inspects the git working tree and the transcript in pure
# shell.  There is no model discretion to skip it and no nested `claude`.
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
        *) qualifying+="$f"$'\n' ;;
    esac
done < <(printf '%s\n' "$changed_files")

if [[ -z "$qualifying" ]]; then
    release
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
