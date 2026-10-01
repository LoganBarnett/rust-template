# mkReleaseOutputsCheck — a `nix flake check` derivation that proves every
# release package ships one native executable under `bin/`, not a script.
#
# `releasePackages` is an attrset of a flake's release-suffixed packages, keyed
# however the caller likes.  The caller gates this check on `system ==
# "x86_64-linux"`: the darwin cross outputs exist only there, it is where the
# release workflow runs, and inspecting anywhere else would build the Windows
# cross set on every contributor's machine.
{
  # A per-system nixpkgs, used for `file` and `runCommand`.
  pkgs,
  # The release-variant packages to inspect, keyed however the caller likes.
  releasePackages,
}:
pkgs.runCommand "release-outputs-native"
# `file` identifies the format of each bin/ entry; it is the whole assertion.
{nativeBuildInputs = [pkgs.file];}
''
  set -euo pipefail
  shopt -s nullglob
  status=0
  for pkg in ${
    pkgs.lib.concatStringsSep " "
    (map toString (pkgs.lib.attrValues releasePackages))
  }; do
    entries=("$pkg"/bin/*)
    if test "''${#entries[@]}" -ne 1; then
      echo "ERROR: $pkg/bin holds ''${#entries[@]} entries, not one:" \
        "''${entries[@]}" >&2
      echo "The release workflow names the asset after the single entry" \
        "under bin/." >&2
      status=1
      continue
    fi
    entry="''${entries[0]}"
    kind=$(file --brief --dereference "$entry")
    case "$kind" in
      ELF*|Mach-O*|PE32*) ;;
      *)
        echo "ERROR: $entry is not a native executable: $kind" >&2
        echo "Whatever sits here ships as the release asset in place of the" \
          "binary." >&2
        status=1
        ;;
    esac
  done
  test "$status" -eq 0
  touch "$out"
''
