#!/usr/bin/env bash
# Gathers the facts about one pull request and judges them with verdict.jq.
#
# A failed lookup fails the script.  The merge never runs on a missing fact.
set -euo pipefail

facts=$(mktemp --directory)

if [ -z "$PR_NUMBER" ]; then
  # The owner keeps a fork's same-named branch from matching.
  PR_NUMBER=$(gh api --method GET "repos/$REPO/pulls" \
    --raw-field state=open \
    --raw-field head="$HEAD_OWNER:$HEAD_BRANCH" \
    --jq '.[0].number // empty')
fi
if [ -z "$PR_NUMBER" ]; then
  echo "No open pull request for $HEAD_OWNER:$HEAD_BRANCH."
  echo "verdict=ignore" >> "$GITHUB_OUTPUT"
  exit 0
fi

gh api "repos/$REPO/pulls/$PR_NUMBER" --jq '{
  number,
  state,
  head_sha: .head.sha,
  head_ref: .head.ref,
  head_repo: .head.repo.full_name,
  base_ref: .base.ref
}' > "$facts/pr.json"
head_ref=$(jq --raw-output '.head_ref' "$facts/pr.json")
head_sha=$(jq --raw-output '.head_sha' "$facts/pr.json")
base_ref=$(jq --raw-output '.base_ref' "$facts/pr.json")

gh api --method GET "repos/$REPO/actions/workflows/$CI_WORKFLOW/runs" \
  --raw-field event=pull_request \
  --raw-field branch="$head_ref" \
  --raw-field per_page=50 \
  --jq '[.workflow_runs[] | {
    id,
    head_sha,
    status,
    conclusion,
    created_at,
    head_repository: .head_repository.full_name
  }]' > "$facts/runs.json"

gh api --paginate "repos/$REPO/issues/$PR_NUMBER/comments" \
  --jq '.[] | {id, body, created_at, user: {login: .user.login}}' \
  | jq --slurp '.' > "$facts/comments.json"

# `-r` keeps backslashes literal and has no long form.
jq --raw-output '[.[].user.login] | unique | .[]' "$facts/comments.json" \
  | while read -r login; do
    permission=$(gh api "repos/$REPO/collaborators/$login/permission" \
      --jq '.permission')
    jq --null-input --arg login "$login" --arg permission "$permission" \
      '{($login): $permission}'
  done \
  | jq --slurp 'add // {}' > "$facts/permissions.json"

# mergeStateStatus can lag a push.  The compare API reads the commit graph
# directly.
behind_by=$(gh api "repos/$REPO/compare/$base_ref...$head_sha" \
  --jq '.behind_by')

jq --null-input \
  --arg marker "$MARKER" \
  --arg comment_id "$COMMENT_ID" \
  --argjson behind_by "$behind_by" \
  --slurpfile pr "$facts/pr.json" \
  --slurpfile runs "$facts/runs.json" \
  --slurpfile comments "$facts/comments.json" \
  --slurpfile permissions "$facts/permissions.json" \
  '{
    marker: $marker,
    trigger_comment_id: ($comment_id | tonumber? // null),
    pr: $pr[0],
    runs: $runs[0],
    comments: $comments[0],
    permissions: $permissions[0],
    behind_by: $behind_by
  }' \
  | jq --from-file "$GITHUB_ACTION_PATH/verdict.jq" \
  | tee "$facts/verdict.json"

jq --raw-output 'to_entries[] | "\(.key)=\(.value)"' "$facts/verdict.json" \
  >> "$GITHUB_OUTPUT"
