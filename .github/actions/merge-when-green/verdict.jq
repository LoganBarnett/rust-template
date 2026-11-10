# Decides what the merge-when-green flow does with one pull request.
# test-merge-when-green.sh covers every branch.
#
# An approval is a maintainer's comment with a line that is exactly the marker.
# It approves the head commit at the time it was written.  The newest
# pull_request CI run dates the last push.  Every push starts one.

def trimmed: gsub("^\\s+|\\s+$"; "");
def at: fromdateiso8601;
def commits: if . == 1 then "1 commit" else "\(.) commits" end;

.marker as $marker
| .permissions as $permissions
| .pr as $pr
| .trigger_comment_id as $asked_id
| $pr.head_sha[0:7] as $sha
| (
    .runs
    # A fork can reuse this branch name.  Its runs are not this build.
    | map(select(.head_repository == $pr.head_repo))
    | sort_by(.created_at | at)
    | last
  ) as $build
| [
    .comments[]
    | select(.body | split("\n") | any(trimmed == $marker))
    | select($permissions[.user.login] | IN("admin", "write"))
  ] as $approvals
| ($approvals | any(.id == $asked_id)) as $asked
| (
    if $pr.state != "open" then
      {verdict: "ignore"}
    elif ($approvals | length) == 0 then
      {verdict: "ignore"}
    elif $build == null or $build.head_sha != $pr.head_sha then
      {
        verdict: "unbuilt",
        message: "No CI run has started for `\($sha)` yet.  Comment `\($marker)` again once it has."
      }
    elif $approvals | all((.created_at | at) <= ($build.created_at | at)) then
      {
        verdict: "stale",
        message: "That `\($marker)` is older than the CI run for `\($sha)`.  Comment `\($marker)` again."
      }
    elif $build.status != "completed" then
      {
        verdict: "armed",
        message: "This will merge once CI passes on `\($sha)`."
      }
    elif $build.conclusion != "success" then
      {
        verdict: "red",
        message: "CI concluded `\($build.conclusion)` on `\($sha)`, so there is nothing to merge."
      }
    elif .behind_by > 0 then
      {
        verdict: "behind",
        message: "CI passed on `\($sha)`, but the branch is \(.behind_by | commits) behind `\($pr.base_ref)`.  Update the branch, then comment `\($marker)` again."
      }
    else
      {verdict: "merge"}
    end
  ) as $outcome
| {
    verdict: $outcome.verdict,
    message: ($outcome.message // ""),
    # A maintainer who just asked always gets an answer.  A branch that fell
    # behind is reported on either trigger.  Nothing else would say why the
    # pull request is still open.
    notify: ($outcome.message != null and ($asked or $outcome.verdict == "behind")),
    pr_number: $pr.number,
    head_sha: $pr.head_sha,
    base_ref: $pr.base_ref
  }
