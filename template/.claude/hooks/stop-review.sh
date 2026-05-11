#!/usr/bin/env bash
# Stop hook: forces a template-compliance review when the assistant
# edited code or config files this turn.
#
# The Claude Code harness fires this when the assistant tries to end
# its turn.  We exit 0 to allow the stop, or emit JSON with
# decision=block to force the assistant to invoke the
# template-compliance review subagent before stopping.
#
# Prose files (.md, .org, .txt, .rst, .adoc) are intentionally ignored
# — review there is not worth the token spend.

set -euo pipefail

input="$(cat)"

stop_hook_active=$(printf '%s' "$input" \
    | jq --raw-output '.stop_hook_active // false')
transcript_path=$(printf '%s' "$input" \
    | jq --raw-output '.transcript_path // empty')

# If we already blocked once this turn and the assistant continued
# without invoking the reviewer, do not loop forever.  Let the turn end.
if [[ "$stop_hook_active" == "true" ]]; then
    exit 0
fi

# Fresh sessions or unexpected input: do nothing.
if [[ -z "$transcript_path" || ! -f "$transcript_path" ]]; then
    exit 0
fi

# Find the JSONL line index of the last *real* user prompt.  A "user"
# entry whose content is a tool_result is a tool response, not a
# prompt; we want the last text prompt so we can scope to "this turn".
last_prompt_idx=$(jq --slurp --raw-input '
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
' "$transcript_path")

# Collect every tool_use call after that prompt.
tool_uses=$(jq --slurp --raw-input --argjson skip "$last_prompt_idx" '
    split("\n")
    | map(select(length > 0))
    | map(fromjson? // empty)
    | .[($skip + 1):]
    | map(select(.type == "assistant"))
    | map(.message.content[]? | select(.type == "tool_use"))
' "$transcript_path")

# Pull out file paths edited via Edit, Write, or MultiEdit.
edited_files=$(printf '%s' "$tool_uses" | jq --raw-output '
    map(select(.name == "Edit" or .name == "Write" or .name == "MultiEdit"))
    | map(.input.file_path // .input.path // empty)
    | .[]
    | select(length > 0)
')

# Drop prose files — only code and config trigger a review.
qualifying_files=""
while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    lower=$(printf '%s' "$f" | tr '[:upper:]' '[:lower:]')
    case "$lower" in
        *.md|*.org|*.txt|*.rst|*.adoc) ;;
        */license|*/license.md|*/license.txt) ;;
        *) qualifying_files+="$f"$'\n' ;;
    esac
done <<< "$edited_files"

if [[ -z "$qualifying_files" ]]; then
    exit 0
fi

# Was the compliance subagent invoked since the last user prompt?
reviewed=$(printf '%s' "$tool_uses" | jq '
    map(select(.name == "Task"
               and .input.subagent_type == "template-compliance"))
    | length
')

if [[ "${reviewed:-0}" -gt 0 ]]; then
    exit 0
fi

# Block.  The reason is shown to the assistant verbatim.
jq --null-input --arg files "$qualifying_files" '{
    decision: "block",
    reason: (
        "Code/config files were modified this turn:\n"
        + $files
        + "\nBefore ending the turn, invoke the template-compliance "
        + "review subagent (Agent tool, "
        + "subagent_type=\"template-compliance\") to verify the work "
        + "follows the conventions in CLAUDE.md and llms.org.  "
        + "Address findings before stopping again.  This hook will "
        + "not block a second time."
    )
}'
