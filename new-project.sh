#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE_DIR="$SCRIPT_DIR/template"

PROJECT_NAME=""
DESCRIPTION=""
CRATES="cli,server"
OUTPUT=""
PUBLIC=false

usage() {
    cat <<EOF
Usage: $(basename "$0") --name <project-name> --output <path> [options]

  --name         Project name, used for directory and package names.
  --output       Destination directory (must be empty or not yet exist).
  --description  One-line project description (optional).
  --crates       Comma-separated binary crates to include (default: cli,server).
                 Available: cli, server.  lib is always included.
  --public       Mark the lib crate as publishable and include the crates.io
                 publish workflow.  Without this flag the lib crate has
                 publish = false and no publish workflow is emitted.

Examples:
  $(basename "$0") --name my-app --output ~/dev/my-app
  $(basename "$0") --name my-svc --output ~/dev/my-svc --crates server --description "HTTP microservice"
  $(basename "$0") --name my-lib --output ~/dev/my-lib --public
EOF
    exit 1
}

# Detect whether the sed in PATH is GNU (supports -i without an extension
# argument) or BSD (requires -i '').
if sed --version 2>/dev/null | grep -q GNU; then
    sed_inplace() { sed -i "$@"; }
else
    sed_inplace() { sed -i '' "$@"; }
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        --name)        PROJECT_NAME="$2"; shift 2 ;;
        --output)      OUTPUT="$2";       shift 2 ;;
        --description) DESCRIPTION="$2"; shift 2 ;;
        --crates)      CRATES="$2";       shift 2 ;;
        --public)      PUBLIC=true;       shift ;;
        -h|--help)     usage ;;
        *) echo "Unknown option: $1" >&2; usage ;;
    esac
done

[[ -z "$PROJECT_NAME" ]] && { echo "Error: --name is required." >&2; usage; }
[[ -z "$OUTPUT" ]]       && { echo "Error: --output is required." >&2; usage; }

if [[ -e "$OUTPUT" ]]; then
    if [[ ! -d "$OUTPUT" ]]; then
        echo "Error: output path exists and is not a directory: $OUTPUT" >&2
        exit 1
    fi
    # Allow a pre-populated directory as long as no template file would
    # overwrite an existing file.  This supports workflows where a project
    # directory is seeded with artifacts (e.g. overview.org) before the
    # template is applied.
    conflicts=()
    while IFS= read -r -d '' template_file; do
        relative="${template_file#"$TEMPLATE_DIR"/}"
        if [[ -e "$OUTPUT/$relative" ]]; then
            conflicts+=("$relative")
        fi
    done < <(find "$TEMPLATE_DIR" -type f -print0)
    if [[ ${#conflicts[@]} -gt 0 ]]; then
        echo "Error: template files conflict with existing files in $OUTPUT:" >&2
        for f in "${conflicts[@]}"; do
            echo "  $f" >&2
        done
        exit 1
    fi
fi

echo "Creating $PROJECT_NAME in $OUTPUT ..."

mkdir -p "$OUTPUT"
cp -r "$TEMPLATE_DIR/." "$OUTPUT/"

# Substitute the placeholder project name throughout all text files.
grep -rl 'rust-template' "$OUTPUT" | while IFS= read -r f; do
    sed_inplace "s/rust-template/$PROJECT_NAME/g" "$f"
done

# Substitute the underscore form used in Rust lib names and `use` statements
# (e.g. `rust_template_server` → `my_project_server`).  Must run after the
# hyphen pass so any hyphen→underscore collisions are already resolved.
PROJECT_NAME_UNDERSCORE="${PROJECT_NAME//-/_}"
grep -rl 'rust_template' "$OUTPUT" | while IFS= read -r f; do
    sed_inplace "s/rust_template/$PROJECT_NAME_UNDERSCORE/g" "$f"
done

# Restore foundation crate references that were mangled by the global
# substitution above.  The foundation crate always keeps its canonical name
# regardless of the downstream project name.
grep -rl "${PROJECT_NAME}-foundation" "$OUTPUT" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/${PROJECT_NAME}-foundation/rust-template-foundation/g" "$f"
done
grep -rl "${PROJECT_NAME_UNDERSCORE}_foundation" "$OUTPUT" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/${PROJECT_NAME_UNDERSCORE}_foundation/rust_template_foundation/g" "$f"
done

# Substitute the placeholder description if one was provided.
if [[ -n "$DESCRIPTION" ]]; then
    grep -rl 'Rust Template - Best-in-class Rust project setup' "$OUTPUT" | while IFS= read -r f; do
        sed_inplace "s/Rust Template - Best-in-class Rust project setup/$DESCRIPTION/g" "$f"
    done
fi

# Prune binary crates that were not requested, removing their directories and
# scrubbing their entries from Cargo.toml and flake.nix.
ALL_BINARY_CRATES=(cli server)
IFS=',' read -ra REQUESTED <<< "$CRATES"

for crate in "${ALL_BINARY_CRATES[@]}"; do
    if [[ ! " ${REQUESTED[*]} " =~ " $crate " ]]; then
        echo "  Removing crate: $crate"

        rm -rf "$OUTPUT/crates/$crate"

        # Strip the workspace member line from Cargo.toml.
        grep -v "\"crates/$crate\"" "$OUTPUT/Cargo.toml" > "$OUTPUT/Cargo.toml.tmp"
        mv "$OUTPUT/Cargo.toml.tmp" "$OUTPUT/Cargo.toml"

        # Strip the workspaceCrates block from flake.nix using the sentinel
        # comments added by this template.
        awk "/# CRATE:$crate:begin/{skip=1; next} \
             /# CRATE:$crate:end/{skip=0; next} \
             !skip" \
            "$OUTPUT/flake.nix" > "$OUTPUT/flake.nix.tmp"
        mv "$OUTPUT/flake.nix.tmp" "$OUTPUT/flake.nix"
    fi
done

if [[ "$PUBLIC" == true ]]; then
    # Remove the publish = false guard from the lib crate so it can be
    # published to crates.io.
    sed_inplace '/^publish = false$/d' "$OUTPUT/crates/lib/Cargo.toml"
else
    # Remove the crates.io publish workflow; it has no use in a private project.
    # Keep CI, branch protection, dependabot, and automerge.
    rm -f "$OUTPUT/.github/workflows/publish.yml"
fi

# Restore reusable workflow references in GitHub Actions callers.  The global
# substitution above mangles `LoganBarnett/rust-template/` to
# `LoganBarnett/$PROJECT_NAME/`; undo that so callers point at the template repo.
grep -rl "LoganBarnett/${PROJECT_NAME}/" "$OUTPUT" 2>/dev/null \
  | while IFS= read -r f; do
    sed_inplace "s|LoganBarnett/${PROJECT_NAME}/|LoganBarnett/rust-template/|g" "$f"
done

# Write template provenance so forward-porting can scope diffs precisely
# (see docs/compliance.org § "Forward-porting template updates").
TEMPLATE_HASH="$(git -C "$SCRIPT_DIR" rev-parse HEAD 2>/dev/null || echo "unknown")"
cat > "$OUTPUT/rust-template.json" <<EOF
{
  "template_sync_hashes": ["$TEMPLATE_HASH"]
}
EOF

# ---------------------------------------------------------------------------
# Register this spawn in config.json so forward-porting can discover it.
# ---------------------------------------------------------------------------
CONFIG="$SCRIPT_DIR/config.json"
if [[ ! -f "$CONFIG" ]]; then
    cp "$SCRIPT_DIR/config.template.json" "$CONFIG"
fi

RESOLVED_OUTPUT="$(cd "$OUTPUT" && pwd)"

jq --arg repo "$PROJECT_NAME" \
   --arg dir  "$RESOLVED_OUTPUT" \
   --arg crates "$CRATES" \
   --arg desc "$DESCRIPTION" \
   --argjson public "$PUBLIC" \
   '.templateSpawns[$repo] = {
       dir: $dir,
       archived: false,
       args: {
           crates: $crates,
           description: $desc,
           public: $public
       }
   }' "$CONFIG" > "$CONFIG.tmp" && mv "$CONFIG.tmp" "$CONFIG"

echo "Done.  Next steps:"
echo "  cd $OUTPUT"
echo "  git init && git add . && git commit -m 'Initial commit'"
echo "  direnv allow   # if using nix + direnv"
