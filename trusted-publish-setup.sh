#!/usr/bin/env bash
set -euo pipefail

# Automates crates.io trusted-publishing setup for every publishable
# workspace crate in a spawned project.  For each crate this does:
#
#   1. Claims the crate name by running `cargo publish --package <name>`
#      (skipped if the crate already exists on crates.io).
#   2. POSTs a GitHub Actions trust configuration to
#      `/api/v1/trustpub/github_configs`, authorizing the project's
#      publish workflow to mint short-lived crates.io tokens via OIDC.
#
# Inputs come from the project's own state — workspace crate names
# from `cargo metadata`, GitHub `owner/repo` from the `origin` remote
# — so the only argument the operator has to supply is the project
# directory.  The crates.io API token comes from the environment and
# is never persisted.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=script-common.sh
source "$SCRIPT_DIR/script-common.sh"

PROJECT_DIR=""
WORKFLOW_FILE="publish.yml"
ENVIRONMENT=""
DRY_RUN=false

usage() {
    cat <<EOF
Usage: $(basename "$0") --project-dir <path> [options]

Sets up crates.io trusted publishing for every publishable workspace
crate in a spawned project.  For each crate, claims the name via
\`cargo publish\` (skipped when the crate already exists), then
registers a GitHub Actions trust configuration authorizing the
project's publish workflow.

  --project-dir    Path to the spawned project directory.  Required.
  --workflow-file  Workflow filename to register (default: publish.yml).
                   Names just the file, not the full path — e.g.
                   \`publish.yml\`, not \`.github/workflows/publish.yml\`.
  --environment    Optional GitHub Actions environment to scope the
                   trust config to.  Leave unset for repo-wide scope.
  --dry-run        Print what would happen without doing it.

CARGO_REGISTRY_TOKEN must be set in the environment.  The token needs
both \`publish-new\` and \`trusted-publishing\` scopes, with crate scope
left unrestricted so it covers every crate the user owns.

Requires \`cargo\`, \`git\`, \`jq\`, and \`curl\` on PATH — the
rust-template devShell provides all four.

Examples:
  CARGO_REGISTRY_TOKEN=... $(basename "$0") --project-dir ~/dev/my-app
  CARGO_REGISTRY_TOKEN=... $(basename "$0") --project-dir ~/dev/my-app --dry-run
EOF
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --project-dir)   PROJECT_DIR="$2";   shift 2 ;;
        --workflow-file) WORKFLOW_FILE="$2"; shift 2 ;;
        --environment)   ENVIRONMENT="$2";   shift 2 ;;
        --dry-run)       DRY_RUN=true;       shift ;;
        -h|--help)       usage ;;
        *) echo "Unknown option: $1" >&2; usage ;;
    esac
done

[[ -z "$PROJECT_DIR" ]] && { echo "Error: --project-dir is required." >&2; usage; }
[[ ! -d "$PROJECT_DIR" ]] && { echo "Error: $PROJECT_DIR is not a directory." >&2; exit 1; }

if [[ -z "${CARGO_REGISTRY_TOKEN:-}" ]]; then
    echo "Error: CARGO_REGISTRY_TOKEN must be set in the environment." >&2
    exit 1
fi

# Resolve the GitHub `owner/repo` from the project's `origin` remote
# rather than the local directory name.  Trust configs key publishing
# rights to the remote on GitHub's side, so the remote URL is the
# authoritative identifier even when the local directory has been
# renamed.  Handles both `git@github.com:owner/repo(.git)` and
# `https://github.com/owner/repo(.git)` forms.
remote=$(git -C "$PROJECT_DIR" remote get-url origin)
owner_repo=$(printf '%s\n' "$remote" \
    | sed --regexp-extended \
        --expression 's|^git@[^:]+:||' \
        --expression 's|^https?://[^/]+/||' \
        --expression 's|\.git$||')
owner=$(printf '%s\n' "$owner_repo" | cut --delimiter=/ --fields=1)
repo=$(printf '%s\n' "$owner_repo" | cut --delimiter=/ --fields=2)

if [[ -z "$owner" || -z "$repo" || "$owner" == "$owner_repo" ]]; then
    echo "Error: could not parse owner/repo from git remote: $remote" >&2
    exit 1
fi

# Collect publishable workspace crates.  `--no-deps` skips the dep
# tree — we only need the workspace member list.  `.publish != []`
# excludes crates marked `publish = false` from the loop, since they
# cannot go on crates.io and have no trust config to register.
crates=()
while IFS= read -r name; do
    crates+=("$name")
done < <(
    cargo metadata --format-version 1 --no-deps \
        --manifest-path "$PROJECT_DIR/Cargo.toml" \
        | jq --raw-output '.packages[] | select(.publish != []) | .name'
)

if [[ ${#crates[@]} -eq 0 ]]; then
    echo "No publishable crates found in $PROJECT_DIR."
    exit 0
fi

echo "Project:  $PROJECT_DIR"
echo "GitHub:   $owner/$repo"
echo "Workflow: $WORKFLOW_FILE"
[[ -n "$ENVIRONMENT" ]] && echo "Env:      $ENVIRONMENT"
echo "Crates:   ${crates[*]}"
[[ "$DRY_RUN" == true ]] && echo "Dry run:  yes"
echo

API_BASE="https://crates.io/api/v1"

# Returns 0 if the crate exists on crates.io, 1 otherwise.  The
# `GET /api/v1/crates/<name>` endpoint is public, so no token is
# attached — the unauthenticated probe avoids burning a token slot
# on a check that is observable to the whole world anyway.
crate_exists() {
    local name="$1"
    local code
    code=$(curl --silent --output /dev/null --write-out '%{http_code}' \
        "$API_BASE/crates/$name")
    [[ "$code" == "200" ]]
}

for crate in "${crates[@]}"; do
    echo "─── $crate ───"

    if crate_exists "$crate"; then
        echo "  Crate already exists on crates.io; skipping cargo publish."
    elif [[ "$DRY_RUN" == true ]]; then
        echo "  [dry-run] would run: cargo publish --package $crate"
    else
        echo "  Running cargo publish --package $crate ..."
        # `if` discards the exit code so `set -e` does not terminate
        # the script when the publish fails — we still want to try
        # registering the trust config (the crate may have been
        # claimed by a prior partial run).
        if cargo publish \
                --manifest-path "$PROJECT_DIR/Cargo.toml" \
                --package "$crate"; then
            echo "  Publish succeeded."
        else
            echo "  cargo publish failed; trust config will still be attempted." >&2
        fi
    fi

    body=$(jq --null-input \
        --arg krate "$crate" \
        --arg owner "$owner" \
        --arg repo "$repo" \
        --arg workflow "$WORKFLOW_FILE" \
        --arg env "$ENVIRONMENT" \
        '{
            github_config: (
                {
                    krate: $krate,
                    repository_owner: $owner,
                    repository_name: $repo,
                    workflow_filename: $workflow
                }
                + (if $env != "" then {environment: $env} else {} end)
            )
        }')

    if [[ "$DRY_RUN" == true ]]; then
        echo "  [dry-run] would POST: $body"
        echo
        continue
    fi

    echo "  Registering trust config ..."
    # `--write-out '\n%{http_code}'` puts the HTTP status on its own
    # final line so we can split status from body without parsing
    # the JSON response shape (which varies between success and
    # error cases).
    response=$(curl --silent --show-error \
        --write-out '\n%{http_code}' \
        --header "Authorization: $CARGO_REGISTRY_TOKEN" \
        --header "Content-Type: application/json" \
        --data "$body" \
        "$API_BASE/trustpub/github_configs")
    http_code=$(printf '%s\n' "$response" | tail --lines=1)
    response_body=$(printf '%s\n' "$response" | sed '$d')

    case "$http_code" in
        2*) echo "  Trust config registered." ;;
        4*) echo "  HTTP $http_code from crates.io (likely already configured):"
            echo "  $response_body" ;;
        *)  echo "  Unexpected HTTP $http_code from crates.io:" >&2
            echo "  $response_body" >&2 ;;
    esac

    echo
done

echo "Done."
