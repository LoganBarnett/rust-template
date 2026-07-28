#!/usr/bin/env bash
# One-shot catch-up for a Dependabot backlog.
#
# Dependabot opens one pull request per dependency bump.  When auto-merge has
# been paused or broken, these pile up, and landing them one at a time is slow
# and conflict-prone — every merge restacks the rest on Cargo.lock.  This
# bundles them instead: it takes the versions Dependabot already resolved,
# replays each bump's manifest change onto a fresh branch off the base branch,
# reconciles Cargo.lock to exactly those versions (it does not re-resolve
# anything), composes the changelog, opens one pull request, and — once that
# PR's CI is green — merges it, landing every safe bump at once.  A bump whose
# own CI is red is left untouched for a human.
#
# It runs as the invoking user, who has push access, so the Dependabot
# bot-command restrictions do not apply.  It is a one-shot: run it to clear a
# backlog, then let per-PR auto-merge handle the steady-state trickle.
#
# The assembly happens in a throwaway git worktree at a fixed path under
# $TMPDIR.  On success the worktree is removed; on failure it is left in place
# so its state can be inspected, and a subsequent run refuses until it is gone.
#
# This script is packaged as a Nix derivation (dependabot-combine.nix) that puts
# every tool it calls on PATH; do not assume anything beyond that set is
# present.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: dependabot-combine [options]

Bundle all passing open Dependabot PRs into one PR and merge it.

  --repo owner/name   Target repository (default: gh's auto-detection).
  --base branch       Base branch to combine onto (default: main).
  --changelog file    Changelog file to append (default: CHANGELOG.org).
  --dry-run           List the PRs that would be combined, then stop.
  --no-merge          Create the combined PR but do not merge it.
  --help              Show this help.
EOF
}

REPO=""
BASE="main"
CHANGELOG="CHANGELOG.org"
DRY_RUN="false"
NO_MERGE="false"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo) REPO="$2"; shift 2 ;;
    --base) BASE="$2"; shift 2 ;;
    --changelog) CHANGELOG="$2"; shift 2 ;;
    --dry-run) DRY_RUN="true"; shift ;;
    --no-merge) NO_MERGE="true"; shift ;;
    --help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# Default the target repo to a github.com remote when --repo is not given, so a
# repo whose `origin` is a non-GitHub mirror (e.g. Gitea) resolves without
# hardcoding an owner — gh's own default is `origin`, which would miss the
# mirror.  Trimming everything up to `github.com` and the trailing `.git` yields
# owner/name from both the ssh (git@github.com:owner/name.git) and https forms.
if [ -z "$REPO" ]; then
  url=$(git remote --verbose \
    | awk '/github\.com/ && /\(push\)$/ {print $2; exit}')
  if [ -n "$url" ]; then
    url="${url##*github.com}"
    url="${url#[:/]}"
    REPO="${url%.git}"
  fi
fi

repo_args=()
if [ -n "$REPO" ]; then
  repo_args=(--repo "$REPO")
fi

# --- Gather the passing Dependabot bump PRs --------------------------------

echo "Finding open Dependabot pull requests..."
# `mapfile -t` reads each line of input into an array element, stripping the
# trailing newline.  The short flag has no long form.
mapfile -t rows < <(
  gh pr list "${repo_args[@]}" --state open --author app/dependabot \
    --limit 100 \
    --json number,title,headRefName,labels \
    --jq '.[] | [
      .number,
      .headRefName,
      ([.labels[].name] | index("dependabot-hold") != null),
      .title
    ] | @tsv'
)

if [ "${#rows[@]}" -eq 0 ]; then
  echo "No open Dependabot PRs; nothing to combine."
  exit 0
fi

numbers=()
# `declare -A` declares an associative array (string-keyed map).  The short flag
# has no long form.
declare -A head_of crate_of from_of to_of
skipped_failing=()
skipped_held=()

for row in "${rows[@]}"; do
  # `read -r` reads raw, leaving backslashes literal instead of treating them as
  # escape characters.  The short flag has no long form.
  IFS=$'\t' read -r number head held title <<<"$row"
  if [ "$held" = "true" ]; then
    skipped_held+=("#$number ($title)")
    continue
  fi
  # A bump is eligible only if its own latest CI is green: `gh pr checks`
  # exits non-zero when any check is failing or still pending.
  if ! gh pr checks "$number" "${repo_args[@]}" >/dev/null 2>&1; then
    skipped_failing+=("#$number ($title)")
    continue
  fi
  # Titles are "Bump <crate> from <old> to <new>"; the crate and versions come
  # straight off that so nothing has to be re-derived.
  if [[ "$title" =~ ^Bump\ ([^\ ]+)\ from\ ([^\ ]+)\ to\ ([^\ ]+) ]]; then
    numbers+=("$number")
    head_of["$number"]="$head"
    crate_of["$number"]="${BASH_REMATCH[1]}"
    from_of["$number"]="${BASH_REMATCH[2]}"
    to_of["$number"]="${BASH_REMATCH[3]}"
  else
    echo "  skip #$number: cannot parse crate/version from: $title" >&2
    skipped_failing+=("#$number (unparseable: $title)")
  fi
done

if [ "${#skipped_held[@]}" -gt 0 ]; then
  echo "Skipping held bumps (dependabot-hold):"
  printf '  %s\n' "${skipped_held[@]}"
fi
if [ "${#skipped_failing[@]}" -gt 0 ]; then
  echo "Leaving bumps whose CI is not green for a human:"
  printf '  %s\n' "${skipped_failing[@]}"
fi

if [ "${#numbers[@]}" -eq 0 ]; then
  echo "No passing Dependabot PRs to combine."
  exit 0
fi

echo "Combining ${#numbers[@]} passing bump(s):"
for number in "${numbers[@]}"; do
  echo "  #$number: ${crate_of[$number]} ${from_of[$number]} -> ${to_of[$number]}"
done

if [ "$DRY_RUN" = "true" ]; then
  echo "(--dry-run: stopping before touching anything.)"
  exit 0
fi

# --- Resolve the GitHub-hosting remote -------------------------------------

# `origin` may be a mirror (this repo's origin is Gitea); when the repo is known
# (passed or auto-detected above), prefer the remote whose URL names it.
remote="origin"
if [ -n "$REPO" ]; then
  # `read -r` reads raw, leaving backslashes literal instead of treating them
  # as escape characters.  The short flag has no long form.
  while read -r name url; do
    case "$url" in
      *"$REPO"*) remote="$name"; break ;;
    esac
  done < <(git remote --verbose | awk '/\(push\)$/ {print $1, $2}')
fi

main_repo=$(git rev-parse --show-toplevel)

# --- Prepare the throwaway worktree at a fixed path ------------------------

worktree="${TMPDIR:-/tmp}/dependabot-combine"
worktree="${worktree%/}"
combine_branch="dependabot-combine"

if [ -e "$worktree" ]; then
  echo "error: a worktree from a previous run is still at:" >&2
  echo "  $worktree" >&2
  echo "It was left behind because that run did not finish cleanly.  Inspect" >&2
  echo "it, then remove it and re-run:" >&2
  echo "  git -C $main_repo worktree remove --force $worktree" >&2
  exit 1
fi

echo "Assembling in throwaway worktree: $worktree"
git fetch "$remote" "$BASE"
# `git -C <path>` runs git as if started in <path> (used throughout to act on
# the worktree without cd-ing); `worktree add -B <branch>` creates or resets
# the branch.  Neither short flag has a long form.
git -C "$main_repo" worktree add -B "$combine_branch" "$worktree" "$remote/$BASE"

# From here on, any failure leaves the worktree in place (no cleanup trap): the
# worktree removal at the very end runs only when everything above it
# succeeded, so a broken run keeps its black box for inspection.

# --- Replay each bump's manifest change ------------------------------------

applied=()
for number in "${numbers[@]}"; do
  head="${head_of[$number]}"
  git -C "$worktree" fetch "$remote" "$head"
  # Replay only the manifest edits (Cargo.toml at any depth); Cargo.lock is
  # regenerated below from the exact target versions, so its diff is dropped.
  manifest=$(
    git -C "$worktree" diff "$remote/$BASE" FETCH_HEAD \
      -- '*.toml' ':(glob)**/Cargo.toml'
  )
  if [ -n "$manifest" ]; then
    if ! printf '%s\n' "$manifest" | git -C "$worktree" apply --index -; then
      echo "  conflict replaying #$number's manifest; leaving it out." >&2
      continue
    fi
  fi
  applied+=("$number")
done

if [ "${#applied[@]}" -eq 0 ]; then
  echo "error: no bumps could be replayed cleanly." >&2
  exit 1
fi

# --- Reconcile Cargo.lock to Dependabot's chosen versions ------------------

echo "Reconciling Cargo.lock..."
landed=()
for number in "${applied[@]}"; do
  crate="${crate_of[$number]}"
  to="${to_of[$number]}"
  # `--precise` pins the exact version Dependabot picked; cargo re-resolves no
  # newer release.  A bump that will not take (e.g. a constraint mismatch) is
  # dropped rather than aborting the whole batch.
  if (cd "$worktree" && cargo update --package "$crate" --precise "$to"); then
    landed+=("$number")
  else
    echo "  cargo update rejected $crate@$to; leaving #$number out." >&2
  fi
done

if [ "${#landed[@]}" -eq 0 ]; then
  echo "error: no bumps survived lockfile reconciliation." >&2
  exit 1
fi

# --- Compose the changelog and commit --------------------------------------

for number in "${landed[@]}"; do
  changelog-roller insert-item \
    --input-file "$worktree/$CHANGELOG" \
    --heading Maintenance \
    --body "Bump ${crate_of[$number]} from ${from_of[$number]} to ${to_of[$number]}" \
    --in-place
done

git -C "$worktree" add --all

subject="combine ${#landed[@]} Dependabot dependency bumps"
{
  echo "$subject"
  echo
  for number in "${landed[@]}"; do
    echo "Bump ${crate_of[$number]} from ${from_of[$number]} to ${to_of[$number]} (#$number)"
  done
} > "$worktree/.combine-msg"
git -C "$worktree" commit --file "$worktree/.combine-msg"
rm --force "$worktree/.combine-msg"

# --- Push, open the PR, and (optionally) merge on green --------------------

echo "Pushing $combine_branch..."
git -C "$worktree" push --force-with-lease "$remote" "$combine_branch"

pr_body=$(
  echo "Combines these passing Dependabot bumps into one PR:"
  echo
  for number in "${landed[@]}"; do
    echo "- #$number: ${crate_of[$number]} ${from_of[$number]} -> ${to_of[$number]}"
  done
)
pr_url=$(
  gh pr create "${repo_args[@]}" \
    --base "$BASE" --head "$combine_branch" \
    --title "$subject" --body "$pr_body"
)
echo "Opened $pr_url"
pr_number="${pr_url##*/}"

if [ "$NO_MERGE" = "true" ]; then
  echo "(--no-merge: leaving $pr_url for you to review and merge.)"
  git -C "$main_repo" worktree remove --force "$worktree"
  exit 0
fi

echo "Waiting for the combined PR's CI..."
while true; do
  summary=$(gh pr checks "$pr_number" "${repo_args[@]}" 2>/dev/null || true)
  if [ -n "$summary" ] && ! printf '%s\n' "$summary" | grep --quiet 'pending'; then
    break
  fi
  sleep 30
done

if printf '%s\n' "$summary" | grep --quiet --ignore-case --extended-regexp '	fail|	failure'; then
  echo "The combined PR's CI is not green; leaving it open for review:"
  echo "  $pr_url"
  git -C "$main_repo" worktree remove --force "$worktree"
  exit 0
fi

echo "CI is green; merging..."
gh pr merge "$pr_number" "${repo_args[@]}" --squash --delete-branch

for number in "${landed[@]}"; do
  gh pr comment "$number" "${repo_args[@]}" \
    --body "Landed via combined PR #$pr_number." || true
  gh pr close "$number" "${repo_args[@]}" || true
done

git -C "$main_repo" worktree remove --force "$worktree"
echo "Merged $pr_url and closed ${#landed[@]} bump PR(s)."
