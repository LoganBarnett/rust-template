#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE_DIR="$SCRIPT_DIR/template"

# shellcheck source=script-common.sh
source "$SCRIPT_DIR/script-common.sh"

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

# Step 1: Copy template skeleton (without crate directories or build artifacts).
mkdir -p "$OUTPUT"
cp -r "$TEMPLATE_DIR/." "$OUTPUT/"
rm -rf "$OUTPUT/crates/cli" "$OUTPUT/crates/server" "$OUTPUT/crates/lib"
rm -rf "$OUTPUT/target"

# Step 2: Global name substitution on skeleton files.
PROJECT_NAME_UNDERSCORE="${PROJECT_NAME//-/_}"

# Hyphen pass: rust-template → project name.
grep -rl 'rust-template' "$OUTPUT" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/rust-template/$PROJECT_NAME/g" "$f"
done || true

# Underscore pass: rust_template → project name underscore form.
grep -rl 'rust_template' "$OUTPUT" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/rust_template/$PROJECT_NAME_UNDERSCORE/g" "$f"
done || true

# Restore foundation crate references mangled by the global substitution.
grep -rl "${PROJECT_NAME}-foundation" "$OUTPUT" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/${PROJECT_NAME}-foundation/rust-template-foundation/g" "$f"
done || true
grep -rl "${PROJECT_NAME_UNDERSCORE}_foundation" "$OUTPUT" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/${PROJECT_NAME_UNDERSCORE}_foundation/rust_template_foundation/g" "$f"
done || true

# Substitute the placeholder description if one was provided.
if [[ -n "$DESCRIPTION" ]]; then
    grep -rl 'Rust Template - Best-in-class Rust project setup' "$OUTPUT" 2>/dev/null | while IFS= read -r f; do
        sed_inplace "s/Rust Template - Best-in-class Rust project setup/$DESCRIPTION/g" "$f"
    done || true
fi

# Restore all LoganBarnett/rust-template references mangled by the global
# substitution.  This covers reusable workflow callers (trailing /) and the
# foundation crate's git URL (trailing .git).
grep -rl "LoganBarnett/${PROJECT_NAME}" "$OUTPUT" 2>/dev/null \
  | while IFS= read -r f; do
    sed_inplace "s|LoganBarnett/${PROJECT_NAME}/|LoganBarnett/rust-template/|g" "$f"
    sed_inplace "s|LoganBarnett/${PROJECT_NAME}\\.git|LoganBarnett/rust-template.git|g" "$f"
done || true

# Step 3: Add crates via crate-add.sh.  lib is always included.
"$SCRIPT_DIR/crate-add.sh" \
    --type lib \
    --project-dir "$OUTPUT" \
    --project-name "$PROJECT_NAME"

IFS=',' read -ra REQUESTED <<< "$CRATES"
for crate in "${REQUESTED[@]}"; do
    "$SCRIPT_DIR/crate-add.sh" \
        --type "$crate" \
        --project-dir "$OUTPUT" \
        --project-name "$PROJECT_NAME"
done

# Step 4: Post-processing.
if [[ "$PUBLIC" == true ]]; then
    # Remove the publish = false guard from the lib crate so it can be
    # published to crates.io.
    sed_inplace '/^publish = false$/d' "$OUTPUT/crates/lib/Cargo.toml"
else
    # Remove the crates.io publish workflow; it has no use in a private project.
    # Keep CI, branch protection, dependabot, and automerge.
    rm -f "$OUTPUT/.github/workflows/publish.yml"
fi

# Write template provenance so subsequent compliance work can scope diffs
# precisely (see docs/compliance.org § "Compliance process").
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
