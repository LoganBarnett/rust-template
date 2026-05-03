#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE_DIR="$SCRIPT_DIR/template"

# shellcheck source=script-common.sh
source "$SCRIPT_DIR/script-common.sh"

CRATE_TYPE=""
PROJECT_DIR=""
CRATE_NAME=""
PROJECT_NAME=""

usage() {
    cat <<EOF
Usage: $(basename "$0") --type <cli|lib|server> --project-dir <path> [options]

  --type          Crate archetype: cli, lib, or server (required).
  --project-dir   Path to the project workspace root (required).
  --name          Crate name; defaults to the type name. Combined with the
                  project name to form the package name: <project>-<name>.
  --project-name  Project name for substitution. Auto-detected from the
                  workspace Cargo.toml if omitted (reads existing member
                  package name patterns). Required when called before any
                  crate exists (i.e. from new-project.sh).

Examples:
  $(basename "$0") --type cli --project-dir ~/dev/my-app
  $(basename "$0") --type server --project-dir ~/dev/my-app --name api
  $(basename "$0") --type cli --project-dir ~/dev/my-app --name worker --project-name my-app
EOF
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --type)         CRATE_TYPE="$2";   shift 2 ;;
        --project-dir)  PROJECT_DIR="$2";  shift 2 ;;
        --name)         CRATE_NAME="$2";   shift 2 ;;
        --project-name) PROJECT_NAME="$2"; shift 2 ;;
        -h|--help)      usage ;;
        *) echo "Unknown option: $1" >&2; usage ;;
    esac
done

# Validate required arguments.
[[ -z "$CRATE_TYPE" ]] && { echo "Error: --type is required." >&2; usage; }
[[ -z "$PROJECT_DIR" ]] && { echo "Error: --project-dir is required." >&2; usage; }

case "$CRATE_TYPE" in
    cli|lib|server) ;;
    *) echo "Error: --type must be cli, lib, or server." >&2; exit 1 ;;
esac

# Default --name to the type.
[[ -z "$CRATE_NAME" ]] && CRATE_NAME="$CRATE_TYPE"

# Auto-detect project name from workspace Cargo.toml if not provided.
if [[ -z "$PROJECT_NAME" ]]; then
    if [[ -f "$PROJECT_DIR/Cargo.toml" ]]; then
        # Look for an existing crate member and extract the project name from
        # its package name (e.g. "my-app-cli" → "my-app").
        existing_member=$(grep -o '"crates/[^"]*"' "$PROJECT_DIR/Cargo.toml" | head -1 | sed 's/"crates\///; s/"//' || true)
        if [[ -n "$existing_member" && -f "$PROJECT_DIR/crates/$existing_member/Cargo.toml" ]]; then
            pkg_name=$(grep '^name = ' "$PROJECT_DIR/crates/$existing_member/Cargo.toml" | head -1 | sed 's/name = "//; s/"//')
            # Strip the crate suffix to get the project name.
            PROJECT_NAME="${pkg_name%-"$existing_member"}"
        fi
    fi
    if [[ -z "$PROJECT_NAME" ]]; then
        echo "Error: --project-name is required (could not auto-detect)." >&2
        exit 1
    fi
fi

PROJECT_NAME_UNDERSCORE="${PROJECT_NAME//-/_}"
CRATE_NAME_UNDERSCORE="${CRATE_NAME//-/_}"

# Fail if the crate directory already exists.
if [[ -d "$PROJECT_DIR/crates/$CRATE_NAME" ]]; then
    echo "Error: crate directory already exists: $PROJECT_DIR/crates/$CRATE_NAME" >&2
    exit 2
fi

# Step 3: Copy the crate template.
echo "  Adding crate: $CRATE_NAME (type: $CRATE_TYPE)"
mkdir -p "$PROJECT_DIR/crates"
cp -r "$TEMPLATE_DIR/crates/$CRATE_TYPE/" "$PROJECT_DIR/crates/$CRATE_NAME/"

# Remove the workspace-deps.toml from the copied crate — it is metadata for
# this script, not a project file.
rm -f "$PROJECT_DIR/crates/$CRATE_NAME/workspace-deps.toml"

# Step 4: Name substitution within the copied crate only.
# Order matters: most-specific patterns first to avoid partial matches.
crate_dir="$PROJECT_DIR/crates/$CRATE_NAME"

# 4a: rust-template-$TYPE → $PROJECT_NAME-$NAME (package/binary names)
grep -rl "rust-template-${CRATE_TYPE}" "$crate_dir" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/rust-template-${CRATE_TYPE}/${PROJECT_NAME}-${CRATE_NAME}/g" "$f"
done || true

# 4b: rust_template_$TYPE → ${PROJECT_NAME_UNDERSCORE}_${NAME_UNDERSCORE} (Rust identifiers)
grep -rl "rust_template_${CRATE_TYPE}" "$crate_dir" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/rust_template_${CRATE_TYPE}/${PROJECT_NAME_UNDERSCORE}_${CRATE_NAME_UNDERSCORE}/g" "$f"
done || true

# 4c: rust-template → $PROJECT_NAME (project-level refs like config file name)
grep -rl 'rust-template' "$crate_dir" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/rust-template/${PROJECT_NAME}/g" "$f"
done || true

# 4d: rust_template → $PROJECT_NAME_UNDERSCORE (project-level Rust identifiers)
grep -rl 'rust_template' "$crate_dir" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/rust_template/${PROJECT_NAME_UNDERSCORE}/g" "$f"
done || true

# 4e: Restore foundation crate references mangled by the substitutions above.
grep -rl "${PROJECT_NAME}-foundation" "$crate_dir" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/${PROJECT_NAME}-foundation/rust-template-foundation/g" "$f"
done || true
grep -rl "${PROJECT_NAME_UNDERSCORE}_foundation" "$crate_dir" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/${PROJECT_NAME_UNDERSCORE}_foundation/rust_template_foundation/g" "$f"
done || true

# Step 5: Insert member into Cargo.toml workspace members list.
sed_inplace "/# CRATE_MEMBERS/i\\
    \"crates/${CRATE_NAME}\"," "$PROJECT_DIR/Cargo.toml"

# Step 6: For binary crates, insert a sentinel-marked block into flake.nix.
if [[ "$CRATE_TYPE" != "lib" ]]; then
    # Determine the description based on the type.
    case "$CRATE_TYPE" in
        cli)    crate_description="CLI application" ;;
        server) crate_description="Server process" ;;
    esac

    sed_inplace "/# CRATE_ENTRIES/i\\
      # CRATE:${CRATE_NAME}:begin\\
      ${CRATE_NAME} = {\\
        name = \"${PROJECT_NAME}-${CRATE_NAME}\";\\
        binary = \"${PROJECT_NAME}-${CRATE_NAME}\";\\
        description = \"${crate_description}\";\\
      };\\
      # CRATE:${CRATE_NAME}:end" "$PROJECT_DIR/flake.nix"
fi

# Step 7: Merge workspace dependencies from workspace-deps.toml.
deps_file="$TEMPLATE_DIR/crates/$CRATE_TYPE/workspace-deps.toml"
if [[ -f "$deps_file" ]]; then
    # Read each dependency line (skip comments and section headers).
    while IFS= read -r line; do
        # Skip blank lines, comments, and section headers.
        [[ -z "$line" || "$line" =~ ^[[:space:]]*# || "$line" =~ ^[[:space:]]*\[ ]] && continue

        # Extract the dependency name (everything before the first =).
        dep_name=$(echo "$line" | sed 's/[[:space:]]*=.*//')

        # Apply project-name substitution to the dep line.
        substituted_line="${line//rust-template/$PROJECT_NAME}"

        # Check if this dep already exists in the project's Cargo.toml.
        if ! grep -q "^${dep_name//\-/\\-}[[:space:]]*=" "$PROJECT_DIR/Cargo.toml" 2>/dev/null; then
            # Also check with the substituted name.
            substituted_dep_name="${dep_name//rust-template/$PROJECT_NAME}"
            if ! grep -q "^${substituted_dep_name//\-/\\-}[[:space:]]*=" "$PROJECT_DIR/Cargo.toml" 2>/dev/null; then
                sed_inplace "/# WORKSPACE_DEPS/i\\
${substituted_line}" "$PROJECT_DIR/Cargo.toml"
            fi
        fi
    done < "$deps_file"
fi

echo "  Crate $CRATE_NAME added successfully."
