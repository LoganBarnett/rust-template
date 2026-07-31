#!/usr/bin/env bash
# One-shot catch-up for a Dependabot backlog.
#
# Dependabot opens one pull request per dependency bump.  When auto-merge has
# been paused or broken, these pile up, and landing them one at a time is slow
# and conflict-prone — every merge restacks the rest on Cargo.lock.  This
# bundles them instead, but it never merges, cherry-picks, or applies the
# Dependabot branches: combining their diffs would force automatic resolution of
# changes that are meant for a human.  It reads each PR purely as a signal of
# *which* packages moved — from the Cargo.lock diff, the one place Dependabot
# cannot reword out from under us — and then bumps those packages itself with
# `cargo update` on a fresh branch off the base.  Cargo does the resolution, so
# any set of bumps lands correctly no matter how they overlap.
#
# It does not chase the exact version Dependabot picked: `cargo update
# --package <name>` advances a package to the newest release its existing
# Cargo.toml constraint already allows, touching only Cargo.lock.  A bump that
# would cross the constraint (a major, or a caret-0.x minor) does not advance
# and is left alone on purpose — those need a human's judgment, which this tool
# must not fake.
#
# A bump whose own build is not proven green is left for a human; so is a PR
# that touches no Cargo.lock (a GitHub Actions bump, say), since it signals no
# package to move.  Once the combined PR's CI is green it is merged.
#
# It runs as the invoking user, who has push access, so the Dependabot
# bot-command restrictions do not apply.  It is a one-shot: run it to clear a
# backlog, then let per-PR auto-merge handle the steady-state trickle.
#
# The target repository is cloned fresh into a throwaway checkout at a fixed
# path under $TMPDIR, so the tool can be pointed at any repo with --repo without
# a local checkout — the repo does not need to carry this tool.  With no --repo
# it targets the GitHub repo of the current checkout.  On success the clone is
# removed; on failure it is left in place so its state can be inspected, and a
# subsequent run for the same repo refuses until it is gone.
#
# This script is packaged as a Nix derivation (dependabot-combine.nix) that puts
# every tool it calls on PATH; do not assume anything beyond that set is
# present.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: dependabot-combine [options]

Bundle all green open Dependabot PRs into one PR and merge it.

  --repo owner/name   Target repository.  Defaults to the GitHub repo of the
                      current checkout; pass it to target any repo from
                      anywhere, with no local checkout required.
  --base branch       Base branch to combine onto (default: main).
  --changelog file    Changelog file to append (default: CHANGELOG.org).
  --dry-run           List the packages that would be updated, then stop.
  --no-merge          Create the combined PR but do not merge it.
  --help              Show this help.
EOF
}

# Read a unified diff on stdin and emit "name<TAB>from<TAB>to" for every
# Cargo.lock [[package]] whose version line changed.  Cargo.lock's canonical
# format keeps `name` and `version` on their own lines inside a [[package]]
# block, so a changed +version paired with the block's name is an unambiguous
# "this package moved" signal.  That is the steady thing this tool leans on
# instead of the PR title, which Dependabot can reword (a commit-message prefix
# already broke an earlier title parse) and which says nothing for a non-cargo
# bump.  Used twice: on a PR's diff to learn which packages to update, and on
# the final diff to report what actually moved.
parse_lock_bumps() {
  awk '
    /^diff --git / { in_lock = ($0 ~ /\/Cargo\.lock b\//) }
    in_lock && /^[+ ]\[\[package\]\]/ { name = ""; from = "" }
    in_lock && /^[+ ]name = / {
      match($0, /"[^"]*"/); name = substr($0, RSTART + 1, RLENGTH - 2)
    }
    in_lock && /^-version = / {
      match($0, /"[^"]*"/); from = substr($0, RSTART + 1, RLENGTH - 2)
    }
    in_lock && /^\+version = / {
      match($0, /"[^"]*"/)
      if (name != "") {
        print name "\t" from "\t" substr($0, RSTART + 1, RLENGTH - 2)
      }
    }
  '
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

# Default the target repo to the current checkout's github.com remote when
# --repo is not given, so a repo whose `origin` is a non-GitHub mirror (e.g.
# Gitea) still resolves without hardcoding an owner.  Trimming everything up to
# `github.com` and the trailing `.git` yields owner/name from both the ssh
# (git@github.com:owner/name.git) and https forms.
if [ -z "$REPO" ]; then
  url=$(git remote --verbose \
    | awk '/github\.com/ && /\(push\)$/ {print $2; exit}')
  if [ -n "$url" ]; then
    url="${url##*github.com}"
    url="${url#[:/]}"
    REPO="${url%.git}"
  fi
fi

if [ -z "$REPO" ]; then
  echo "error: no target repository.  Pass --repo owner/name, or run inside" >&2
  echo "a checkout whose remotes include a github.com URL." >&2
  exit 1
fi

repo_args=(--repo "$REPO")

# --- Gather the packages the green Dependabot PRs signal --------------------

echo "Finding open Dependabot pull requests in $REPO..."
# `mapfile -t` reads each line of input into an array element, stripping the
# trailing newline.  The short flag has no long form.
mapfile -t rows < <(
  gh pr list "${repo_args[@]}" --state open --author app/dependabot \
    --limit 100 \
    --json number,title \
    --jq '.[] | [.number, .title] | @tsv'
)

if [ "${#rows[@]}" -eq 0 ]; then
  echo "No open Dependabot PRs; nothing to combine."
  exit 0
fi

numbers=()
# `declare -A` declares an associative array (string-keyed map), used here as a
# set of package names to update.  The `=()` marks it set, so counting an empty
# map (no green PRs) does not trip `set -u`'s unbound-variable check the way a
# bare `declare -A` does.  The short flag has no long form.
declare -A want_update=()
skipped_unproven=()

for row in "${rows[@]}"; do
  # `read -r` reads raw, leaving backslashes literal instead of treating them as
  # escape characters.  The short flag has no long form.
  IFS=$'\t' read -r number title <<<"$row"
  # A bump is eligible only if its own build is proven green — it has checks and
  # every one concluded success.  A bump that is merely behind main still
  # qualifies (its green build is what counts, not whether it can fast-forward).
  # Every other state is held with the reason why, so the human sees not just
  # which bumps were left but why each one was: a failing check, checks still
  # running, or — the unvalidated case — no checks at all.  (`.conclusion` is a
  # check run's result and `.state` a status context's; SUCCESS/NEUTRAL/SKIPPED
  # are the non-failing terminal states; a failure anywhere beats a pending.)
  status=$(
    gh pr view "$number" "${repo_args[@]}" --json statusCheckRollup \
      --jq '(.statusCheckRollup // [])
            | if length == 0 then "none"
              elif any((.conclusion // .state // "")
                       | . == "FAILURE" or . == "ERROR" or . == "TIMED_OUT"
                         or . == "STARTUP_FAILURE") then "failed"
              elif all((.conclusion // .state // "")
                       | . == "SUCCESS" or . == "NEUTRAL" or . == "SKIPPED")
                then "green"
              else "pending"
              end' 2>/dev/null || echo unreadable
  )
  if [ "$status" != "green" ]; then
    case "$status" in
      failed) reason="a check failed" ;;
      pending) reason="checks still running" ;;
      none) reason="no checks have run" ;;
      *) reason="checks could not be read" ;;
    esac
    skipped_unproven+=("#$number ($title) — $reason")
    continue
  fi
  # Take the PR only as a signal of which packages moved in its Cargo.lock — not
  # as a diff to apply.  A PR that touches no Cargo.lock (a GitHub Actions bump,
  # say) signals nothing and is left for a human.
  mapfile -t names < <(
    gh pr diff "$number" "${repo_args[@]}" | parse_lock_bumps | cut --fields=1
  )
  if [ "${#names[@]}" -eq 0 ]; then
    skipped_unproven+=("#$number ($title) — no Cargo.lock change to apply")
    continue
  fi
  numbers+=("$number")
  for name in "${names[@]}"; do
    want_update["$name"]=1
  done
done

if [ "${#skipped_unproven[@]}" -gt 0 ]; then
  echo "Leaving these bumps for a human:"
  printf '  %s\n' "${skipped_unproven[@]}"
fi

if [ "${#want_update[@]}" -eq 0 ]; then
  echo "No green Dependabot PRs with a Cargo.lock bump to combine."
  exit 0
fi

echo "Updating ${#want_update[@]} package(s) signalled by ${#numbers[@]} PR(s):"
for name in "${!want_update[@]}"; do
  echo "  $name"
done

if [ "$DRY_RUN" = "true" ]; then
  echo "(--dry-run: would cargo-update the above; cross-constraint bumps hold.)"
  exit 0
fi

# --- Clone the target repo into the throwaway checkout ----------------------

clone_root="${TMPDIR:-/tmp}/dependabot-combine"
clone_root="${clone_root%/}"
# owner/name -> owner-name, so each target repo gets its own inspectable clone.
clone_dir="$clone_root/${REPO//\//-}"
combine_branch="dependabot-combine"

if [ -e "$clone_dir" ]; then
  echo "error: a clone from a previous run is still at:" >&2
  echo "  $clone_dir" >&2
  echo "It was left behind because that run did not finish cleanly.  Inspect" >&2
  echo "it, then remove it and re-run:" >&2
  echo "  rm --recursive --force $clone_dir" >&2
  exit 1
fi

echo "Cloning $REPO into throwaway checkout: $clone_dir"
mkdir --parents "$clone_root"
gh repo clone "$REPO" "$clone_dir"
# `git -C <path>` runs git as if started in <path>, used throughout to act on
# the clone without cd-ing; `checkout -B` create-or-resets the combine branch
# onto the freshly fetched base.  Neither short flag has a long form.
git -C "$clone_dir" fetch origin "$BASE"
git -C "$clone_dir" checkout -B "$combine_branch" "origin/$BASE"

# From here on, any failure leaves the clone in place (no cleanup trap): the
# clone removal at the very end runs only when everything above it succeeded, so
# a broken run keeps its black box for inspection.

# --- Bump each signalled package ourselves ---------------------------------

# No diff is applied and no version is chased.  `cargo update --package <name>`
# (no `--precise`) advances the flagged package to the newest release its
# existing Cargo.toml constraint already allows and rewrites only Cargo.lock; a
# cross-constraint bump simply does not advance.  Cargo owns the resolution, so
# overlapping bumps compose correctly.
echo "Updating the signalled packages with cargo..."
for name in "${!want_update[@]}"; do
  if ! (cd "$clone_dir" && cargo update --package "$name"); then
    echo "  cargo update could not move $name; leaving it out." >&2
  fi
done

# Whatever actually moved is exactly the Cargo.lock diff; a package pinned by a
# cross-constraint requirement did not advance and simply is not here.
mapfile -t bumps < <(
  git -C "$clone_dir" diff -- Cargo.lock ':(glob)**/Cargo.lock' | parse_lock_bumps
)
if [ "${#bumps[@]}" -eq 0 ]; then
  echo "Nothing moved: every signalled package was already current or would"
  echo "need a manual, cross-constraint bump.  Leaving them for a human."
  rm --recursive --force "$clone_dir"
  exit 0
fi

echo "Landed ${#bumps[@]} bump(s):"
for bump in "${bumps[@]}"; do
  IFS=$'\t' read -r name from to <<<"$bump"
  echo "  $name $from -> $to"
done

# --- Compose the changelog and commit --------------------------------------

# Only append a changelog entry when the target actually carries the file: a
# repo pointed at with --repo may not use one (or may use a different format),
# and the bump commit still stands without it.
if [ -f "$clone_dir/$CHANGELOG" ]; then
  for bump in "${bumps[@]}"; do
    IFS=$'\t' read -r name from to <<<"$bump"
    changelog-roller insert-item \
      --input-file "$clone_dir/$CHANGELOG" \
      --heading Maintenance \
      --body "Bump $name from $from to $to" \
      --in-place
  done
else
  echo "No $CHANGELOG in $REPO; skipping the changelog entry."
fi

git -C "$clone_dir" add --all

subject="combine ${#bumps[@]} Dependabot dependency bumps"
{
  echo "$subject"
  echo
  for bump in "${bumps[@]}"; do
    IFS=$'\t' read -r name from to <<<"$bump"
    echo "Bump $name from $from to $to"
  done
} > "$clone_dir/.combine-msg"
git -C "$clone_dir" commit --file "$clone_dir/.combine-msg"
rm --force "$clone_dir/.combine-msg"

# --- Push, open the PR, and (optionally) merge on green --------------------

echo "Pushing $combine_branch..."
git -C "$clone_dir" push --force-with-lease origin "$combine_branch"

pr_body=$(
  echo "Combines these Dependabot-signalled bumps into one PR:"
  echo
  for bump in "${bumps[@]}"; do
    IFS=$'\t' read -r name from to <<<"$bump"
    echo "- $name $from -> $to"
  done
  echo
  printf 'Source PRs:'
  printf ' #%s' "${numbers[@]}"
  echo
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
  rm --recursive --force "$clone_dir"
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
  rm --recursive --force "$clone_dir"
  exit 0
fi

echo "CI is green; merging..."
gh pr merge "$pr_number" "${repo_args[@]}" --squash --delete-branch

for number in "${numbers[@]}"; do
  gh pr comment "$number" "${repo_args[@]}" \
    --body "Landed via combined PR #$pr_number." || true
  gh pr close "$number" "${repo_args[@]}" || true
done

rm --recursive --force "$clone_dir"
echo "Merged $pr_url and closed ${#numbers[@]} bump PR(s)."
