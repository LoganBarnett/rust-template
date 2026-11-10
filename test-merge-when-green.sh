#!/usr/bin/env bash
# Tests the merge-when-green verdict in .github/actions/merge-when-green/.
#
# No live pull request is needed.  Each case edits a passing baseline of facts
# with a jq expression.
#
# The stale approvals matter most.  Any push after a `/merge` must void it.
# That includes a force-push back to a commit that was already building.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERDICT="$SCRIPT_DIR/.github/actions/merge-when-green/verdict.jq"

# A maintainer approved commit aaaaaaa five minutes into its build, the build
# passed, and the branch is up to date.
BASELINE='{
  "marker": "/merge",
  "trigger_comment_id": null,
  "pr": {
    "number": 7,
    "state": "open",
    "head_sha": "aaaaaaa1",
    "head_repo": "owner/repo",
    "base_ref": "main"
  },
  "runs": [
    {
      "id": 1,
      "head_sha": "aaaaaaa1",
      "status": "completed",
      "conclusion": "success",
      "created_at": "2026-10-01T10:00:00Z",
      "head_repository": "owner/repo"
    }
  ],
  "comments": [
    {
      "id": 50,
      "body": "/merge",
      "created_at": "2026-10-01T10:05:00Z",
      "user": {"login": "logan"}
    }
  ],
  "permissions": {"logan": "admin"},
  "behind_by": 0
}'

fail=0

# expect NAME VERDICT NOTIFY EDIT applies the jq expression EDIT to the
# baseline.  The result must be VERDICT and NOTIFY.
expect() {
  local name=$1 verdict=$2 notify=$3 edit=$4
  local got
  got=$(jq "$edit" <<<"$BASELINE" \
    | jq --from-file "$VERDICT" \
    | jq --raw-output '"\(.verdict) \(.notify)"') || got="<jq failed>"
  if [ "$got" = "$verdict $notify" ]; then
    echo "ok: $name"
  else
    echo "FAIL: $name: expected '$verdict $notify', got '$got'"
    fail=1
  fi
}

echo "== merges =="
expect "approved mid-build, then green" merge false '.'
expect "approved after the build went green" merge false \
  '.trigger_comment_id = 50'
expect "marker on its own line of a longer comment" merge false \
  '.comments[0].body = "Looks right.\r\n  /merge\r\n"'
expect "write access is enough" merge false \
  '.permissions.logan = "write"'
expect "a newer run on a fork's same-named branch is not this build" \
  merge false \
  '.runs += [{
    id: 2, head_sha: "bbbbbbb2", status: "in_progress", conclusion: null,
    created_at: "2026-10-01T10:30:00Z", head_repository: "mallory/repo"
  }]'

echo "== stale approvals =="
expect "push after the approval" stale false \
  '.pr.head_sha = "ccccccc3"
  | .runs += [{
    id: 3, head_sha: "ccccccc3", status: "completed", conclusion: "success",
    created_at: "2026-10-01T10:20:00Z", head_repository: "owner/repo"
  }]'
expect "force-push back to a commit that was already building" stale false \
  '.runs += [
    {
      id: 4, head_sha: "bbbbbbb2", status: "completed", conclusion: "success",
      created_at: "2026-10-01T10:02:00Z", head_repository: "owner/repo"
    },
    {
      id: 5, head_sha: "aaaaaaa1", status: "in_progress", conclusion: null,
      created_at: "2026-10-01T10:10:00Z", head_repository: "owner/repo"
    }
  ]'
expect "approval written before the build started" stale false \
  '.comments[0].created_at = "2026-10-01T09:59:00Z"'
expect "head moved and no build has started" unbuilt false \
  '.pr.head_sha = "ddddddd4"'
expect "head moved, and the maintainer who just asked is told" unbuilt true \
  '.pr.head_sha = "ddddddd4" | .trigger_comment_id = 50'

echo "== not an approval =="
expect "commenter with read access" ignore false \
  '.permissions.logan = "read"'
expect "commenter with no recorded permission" ignore false \
  '.permissions = {}'
expect "marker inside a sentence" ignore false \
  '.comments[0].body = "please /merge this"'
expect "closed pull request" ignore false \
  '.pr.state = "closed"'

echo "== approved, not mergeable =="
expect "build still running" armed false \
  '.runs[0].status = "in_progress" | .runs[0].conclusion = null'
expect "build still running, and the maintainer who just asked is told" \
  armed true \
  '.runs[0].status = "in_progress" | .runs[0].conclusion = null
  | .trigger_comment_id = 50'
expect "build failed" red false \
  '.runs[0].conclusion = "failure"'
expect "behind the base" behind true \
  '.behind_by = 2'

if [ "$fail" -ne 0 ]; then
  echo "FAILED"
  exit 1
fi
echo "PASSED"
