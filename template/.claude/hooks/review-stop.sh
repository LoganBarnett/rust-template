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
#
# Two further release valves keep the gate off turns that altered nothing.  In
# plan mode the assistant is read-only by construction, so the gate stands
# aside — which also makes switching to plan mode the way to step out of a
# gated task and into a discussion.  And a turn that leaves the qualifying
# working-tree content byte-identical to what the gate last released on has
# nothing new to review: the gate fingerprints that content on release and
# releases again on a match, so a discussion turn after reviewed-but-uncommitted
# edits does not demand a fresh review.  New edits change the fingerprint and
# re-arm it.
#
# Per-session state lives in three small files under TMPDIR: the review round
# counter, the clippy round counter, and the fingerprint last released on.

set -euo pipefail

# Consecutive blocks allowed within one turn before the gate gives up.  The
# first block is usually just "you have not run the reviewer yet", so this
# leaves a few rounds for actually resolving findings.
MAX_ROUNDS=4

input="$(cat)"
session_id="$(printf '%s' "$input" | jq --raw-output '.session_id // "unknown"')"
transcript_path="$(printf '%s' "$input" \
    | jq --raw-output '.transcript_path // empty')"
permission_mode="$(printf '%s' "$input" \
    | jq --raw-output '.permission_mode // empty')"

state_file="${TMPDIR:-/tmp}/review-stop.${session_id}.state"
reviewed_file="${TMPDIR:-/tmp}/review-stop.${session_id}.reviewed"

# Release the gate (allow the stop) and clear any per-turn round state.
release() {
    rm --force "$state_file" 2>/dev/null || true
    exit 0
}

# Release after a verdict, recording the fingerprint of what was released so a
# later Stop that finds the same content need not review it again.  Only the
# verdict paths use this; the other releases (plan mode, nothing qualifying, no
# transcript) reviewed nothing and record nothing.
release_reviewed() {
    printf '%s\n' "$fingerprint" > "$reviewed_file"
    release
}

# Outside a git work tree, or with no transcript, there is nothing to gate.
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    release
fi
if [[ -z "$transcript_path" || ! -f "$transcript_path" ]]; then
    release
fi

# Plan mode is read-only by construction: nothing this turn touched the working
# tree, and whatever already sits there was gated when it was made.  Blocking
# here would only drag the assistant out of a discussion to re-run a review of
# changes it did not make.  A harness that predates the field sends nothing,
# and the gate runs as before.
if [[ "$permission_mode" == "plan" ]]; then
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
# `read -r` (here and below) takes the line raw, without backslash processing;
# the builtin has no long-form spelling of the flag.
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

# 1a. Has anything changed since the gate last released on a verdict?  The
#     fingerprint covers the qualifying files' current content only — one line
#     per path with its blob hash, hashed together — so prose edits do not
#     re-arm the gate and staged-versus-unstaged makes no difference.  A file
#     that no longer exists (a deletion) hashes as "absent".  With the fixture
#     override in place this runs over the fixture's paths just the same, so a
#     different fixture is a different fingerprint.  Checked before the clippy
#     gate so a discussion turn never pays for a compile.
fingerprint="$(
    while IFS= read -r f; do
        [[ -z "$f" ]] && continue
        printf '%s %s\n' "$f" \
            "$(git hash-object -- "$f" 2>/dev/null || printf 'absent')"
    done <<< "$qualifying" | git hash-object --stdin
)"
if [[ -f "$reviewed_file" ]] \
    && [[ "$(cat "$reviewed_file")" == "$fingerprint" ]]; then
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
                # The opening phrase is matched verbatim by the filters in
                # steps 2 and 3, which is how this block is recognised when it
                # comes back as a user turn.  Rewording it without updating
                # them reintroduces the unbounded spin.
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
#
#    Two kinds of user text turn are machine-authored rather than prompts: this
#    gate's own block reason, re-injected after a block, and the notification a
#    background subagent posts when it finishes.  Counting either as a fresh
#    prompt moves the turn boundary past the review that just ran, hiding its
#    verdict, and resets the round counter in step 5 — so the gate can neither
#    converge nor give up, and spins without bound.  Both are recognised by
#    their marker text and skipped.
last_prompt_idx="$(jq --slurp --raw-input '
    # Marker tests run against the whole serialized entry rather than a modelled
    # content shape.  Machine-authored text reaches the transcript by several
    # routes — a text block, a tool_result whose text sits in .content, or a
    # toolUseResult field alongside .message entirely — and a reader that models
    # one of them silently sees "" for the others.  Only the marker matters, so
    # where it sits does not.
    def machine_authored:
      tostring
      | test("This gate releases only when the review reports")
        or test("clippy reported problems on the Rust changes this turn")
        or test("<task-notification>");
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
        and ((.value | machine_authored) | not)
      ))
    | (last // {key: -1}).key
' "$transcript_path")"
last_prompt_idx="${last_prompt_idx:--1}"

# 3. Read the verdict of the most recent template-compliance review since that
#    prompt.  Each review Task is matched by id to its tool_result, and the
#    machine-readable COMPLIANCE: line the subagent emits is read from it.
#
#    Where subagents run in the background, that tool_result holds only launch
#    metadata and the verdict arrives later as a task-notification carrying no
#    tool_use_id to match on.  How that notification is recorded depends on
#    what was in flight when it landed: with a tool call running it rides
#    inside the next user turn, but with nothing running it is queued and
#    recorded as a queued_command attachment plus the queue-operation
#    bookkeeping around it — entries that are not user turns at all.  All of
#    those shapes are collected, and only texts that actually carry a
#    COMPLIANCE: line count as verdicts — which also keeps the launch metadata
#    from being mistaken for a report of findings.
#
#    The hook_authored filter is load-bearing for one specific shape: a block
#    reason quotes the findings text it is reporting, so a re-injected block
#    carries both a task-notification tag and the words "COMPLIANCE: PASS" from
#    this file's own explanation of the rule.  Admitting it would release the
#    gate on the strength of the gate's own prose.
review="$(jq --slurp --raw-input --argjson skip "$last_prompt_idx" '
    def message_text:
      if type == "string" then .
      elif type == "array" then (map(.text? // "") | join("\n"))
      else "" end;
    # See step 2 on why this reads the whole entry rather than a content shape.
    def hook_authored:
      tostring
      | test("This gate releases only when the review reports")
        or test("clippy reported problems on the Rust changes this turn");
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
         | ($r.content | message_text) ]) as $results
    # Only an entry the harness or the human wrote can deliver a verdict —
    # never the assistant, whose prose quotes "COMPLIANCE: PASS" whenever it
    # discusses this gate.  The admitted types are the ones a real transcript
    # records a background notification under (see the step comment above);
    # they are named rather than taken as "anything but assistant" because a
    # compaction summary is assistant-written under yet another type.
    | ([ $all[]
         | select((.type == "user"
                   or .type == "queue-operation"
                   or (.type == "attachment"
                       and .attachment.type == "queued_command"))
                  and (hook_authored | not))
         | tostring
         | select(test("<task-notification>")) ])
      as $notifications
    | ([ ($results + $notifications)[]
         | select(test("COMPLIANCE:")) ]) as $verdicts
    | if ($verdicts | length) == 0 then {verdict: "none", text: ""}
      elif ($verdicts[-1] | test("COMPLIANCE:\\s*PASS")) then
        {verdict: "pass", text: $verdicts[-1]}
      else {verdict: "findings", text: $verdicts[-1]} end
' "$transcript_path")"

verdict="$(printf '%s' "$review" | jq --raw-output '.verdict')"

# 4. A clean review releases the gate immediately, and records what it
#    released on so an unchanged tree next turn is not reviewed again.
if [[ "$verdict" == "pass" ]]; then
    release_reviewed
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
    # The fingerprint is recorded here too: the human has now seen those
    # findings, and blocking again next turn on identical content would be
    # nagging rather than gating.  Any new edit re-arms the gate.
    printf 'template-compliance: releasing after %d unresolved rounds\n' \
        "$MAX_ROUNDS" >&2
    release_reviewed
fi

printf '%s %s\n' "$last_prompt_idx" "$count" > "$state_file"

# Both reasons below close with the same sentence, and the filters in steps 2
# and 3 match it verbatim to recognise this block when it returns as a user
# turn.  Reword it in one place without the other and the gate spins.
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
