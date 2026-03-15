#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE_DIR="$SCRIPT_DIR/template"

PROJECT_NAME=""
DESCRIPTION=""
CRATES="cli,web"
OUTPUT=""

usage() {
    cat <<EOF
Usage: $(basename "$0") --name <project-name> --output <path> [options]

  --name         Project name, used for directory and package names.
  --output       Destination directory (must not already exist).
  --description  One-line project description (optional).
  --crates       Comma-separated binary crates to include (default: cli,web).
                 Available: cli, web.  lib is always included.

Examples:
  $(basename "$0") --name my-app --output ~/dev/my-app
  $(basename "$0") --name my-svc --output ~/dev/my-svc --crates web --description "HTTP microservice"
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
        -h|--help)     usage ;;
        *) echo "Unknown option: $1" >&2; usage ;;
    esac
done

[[ -z "$PROJECT_NAME" ]] && { echo "Error: --name is required." >&2; usage; }
[[ -z "$OUTPUT" ]]       && { echo "Error: --output is required." >&2; usage; }

if [[ -e "$OUTPUT" ]]; then
    echo "Error: output path already exists: $OUTPUT" >&2
    exit 1
fi

echo "Creating $PROJECT_NAME in $OUTPUT ..."

cp -r "$TEMPLATE_DIR" "$OUTPUT"

# Substitute the placeholder project name throughout all text files.
grep -rl 'rust-template' "$OUTPUT" | while IFS= read -r f; do
    sed_inplace "s/rust-template/$PROJECT_NAME/g" "$f"
done

# Substitute the placeholder description if one was provided.
if [[ -n "$DESCRIPTION" ]]; then
    grep -rl 'Rust Template - Best-in-class Rust project setup' "$OUTPUT" | while IFS= read -r f; do
        sed_inplace "s/Rust Template - Best-in-class Rust project setup/$DESCRIPTION/g" "$f"
    done
fi

# Prune binary crates that were not requested, removing their directories and
# scrubbing their entries from Cargo.toml and flake.nix.
ALL_BINARY_CRATES=(cli web)
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

echo "Done.  Next steps:"
echo "  cd $OUTPUT"
echo "  git init && git add . && git commit -m 'Initial commit'"
echo "  direnv allow   # if using nix + direnv"
