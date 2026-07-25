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
# cp short flags (no long forms on BSD cp, so they cannot be spelled out):
#   -R  recurse into directories
#   -P  preserve symlinks instead of dereferencing them; required so that
#       e.g. template/CLAUDE.md (a symlink to llms.org) ships as a symlink
#       in the spawned project rather than as a duplicated file that can
#       drift from its source.
cp -RP "$TEMPLATE_DIR/." "$OUTPUT/"
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

# Restore rust-template.json manifest references mangled by the global
# substitution.  Unlike other rust-template names, the manifest keeps its
# literal filename in every spawn (the reusable release workflow reads a fixed
# rust-template.json, and the emitted flake.nix reads the same file for the
# windows-msvc flag), so any <project>.json the substitution produced from it is
# put back.  The template has no legitimate <project>.json of its own, so this
# is unambiguous.
grep -rl "${PROJECT_NAME}\.json" "$OUTPUT" 2>/dev/null | while IFS= read -r f; do
    sed_inplace "s/${PROJECT_NAME}\.json/rust-template.json/g" "$f"
done || true

# Substitute the placeholder description if one was provided.
if [[ -n "$DESCRIPTION" ]]; then
    grep -rl 'Rust Template - Best-in-class Rust project setup' "$OUTPUT" 2>/dev/null | while IFS= read -r f; do
        sed_inplace "s/Rust Template - Best-in-class Rust project setup/$DESCRIPTION/g" "$f"
    done || true
fi

# Restore all LoganBarnett/rust-template references mangled by the global
# substitution.  This covers reusable workflow callers (trailing /), the
# foundation crate's git URL (trailing .git), and the foundation flake
# input URL (bare, ending with a quote).
grep -rl "LoganBarnett/${PROJECT_NAME}" "$OUTPUT" 2>/dev/null \
  | while IFS= read -r f; do
    sed_inplace "s|LoganBarnett/${PROJECT_NAME}/|LoganBarnett/rust-template/|g" "$f"
    sed_inplace "s|LoganBarnett/${PROJECT_NAME}\\.git|LoganBarnett/rust-template.git|g" "$f"
    sed_inplace "s|LoganBarnett/${PROJECT_NAME}\"|LoganBarnett/rust-template\"|g" "$f"
done || true

# Write template provenance before adding crates so each crate-add invocation
# can enrich it with that crate's workspace-inventory entry.  The hashes let
# subsequent compliance work scope diffs precisely (see docs/compliance.org
# § "Compliance process").
TEMPLATE_HASH="$(git -C "$SCRIPT_DIR" rev-parse HEAD 2>/dev/null || echo "unknown")"
# `apple-frameworks` is the opt-in that wires the Apple SDK into the darwin
# cross-build (via foundation.lib.pkgsUnfreeFor in the emitted flake).  It is
# seeded false here and flipped true by crate-add.sh when a server crate is
# added — whose foundation `auth` feature links the macOS Security /
# SystemConfiguration / CoreFoundation frameworks — so the flag tracks framework
# linking whether the server is in the initial crate set or added later.
cat > "$OUTPUT/rust-template.json" <<EOF
{
  "template_sync_hashes": ["$TEMPLATE_HASH"],
  "windows-msvc": false,
  "apple-frameworks": false
}
EOF

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

# Step 4: Post-processing.  The publish workflow ships in every spawn; whether
# it reaches crates.io is governed by each crate's publish destination list,
# not by deleting the file.  A public project publishes its library, so point
# that crate's destination at crates.io; a private project keeps the empty
# list, and the workflow's guard skips the crates.io step while still bumping
# the version, rolling the changelog, and tagging on merge.
if [[ "$PUBLIC" == true ]]; then
    sed_inplace 's/^publish = \[\]$/publish = ["crates-io"]/' "$OUTPUT/crates/lib/Cargo.toml"
fi

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

# ---------------------------------------------------------------------------
# Format the emitted files.  Substitutions and crate-add expansions can
# leave imports out of alphabetical order (e.g. `rust_template_lib::`
# becomes `${project}_lib::` while `rust_template_foundation::` is
# restored, so their relative ordering depends on the project's first
# letter) and similar non-canonical residue that no amount of careful
# template authoring can avoid.  Running treefmt here means spawned
# projects come out formatter-clean, and `treefmt --fail-on-change` in
# their CI does not trip on day-zero artifacts.
#
# `treefmt` and the per-language formatter binaries are expected on
# PATH -- via the rust-template devShell, a system install, or however
# the user manages tooling.  A warning is printed and the spawn
# continues if treefmt is missing or fails; the user can re-run
# `treefmt` manually under their own environment to clean up.
# ---------------------------------------------------------------------------
echo "Formatting emitted files ..."
if ! (cd "$OUTPUT" && treefmt) > /dev/null 2>&1; then
    echo "Warning: treefmt failed; emitted files may be unformatted." >&2
fi

echo "Done.  Next steps:"
echo "  cd $OUTPUT"
echo "  git init && git add . && git commit -m 'Initial commit'"
echo "  direnv allow   # if using nix + direnv"
