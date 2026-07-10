# mkWindowsSmokeCheck — a `nix flake check` derivation that proves the x86_64
# Windows cross binaries actually execute, by running each one under wine on the
# Linux builder.
#
# The Windows cross build (mkWindowsCrossPackages) produces PE executables the
# Linux builder cannot run, so a plain build only proves the toolchain linked
# something — not that the result launches.  wine closes that gap for the
# x86_64 binaries: it runs a win64 PE on an x86_64 Linux host, so invoking each
# `.exe` with `--help` (which any clap-based CLI answers with exit 0) is a
# faithful "does it start and run" smoke test.
#
# Only x86_64 is covered.  wine cannot execute an aarch64 PE, and no
# arm64-Windows runner exists to run one, so the aarch64 Windows binaries are
# build-verified only.  wine on Apple Silicon is unreliable, so the caller gates
# this check to x86_64-linux — it is a CI-side guard, not something a
# contributor's Mac runs.
#
# `windowsPackages` is an attrset of the x86_64 Windows cross derivations to
# smoke-test (the `-x86_64-windows`-suffixed subset of a flake's packages); only
# the values (derivations exposing `$out/bin/<binary>.exe`) are used.  Because
# the Windows helper is host-agnostic, that subset is non-empty on every host,
# so the caller gates this check on `system == "x86_64-linux"` (the one host
# where wine runs a win64 PE reliably) rather than on emptiness.
{
  # A per-system nixpkgs, used for `wine64` and `runCommand`.
  pkgs,
  # The x86_64 Windows cross packages to run, keyed however the caller likes.
  windowsPackages,
}:
pkgs.runCommand "windows-smoke-wine"
{nativeBuildInputs = [pkgs.wine64];}
''
  set -euo pipefail
  shopt -s nullglob
  # wine needs a writable HOME and prefix; crane's HOME=/homeless-shelter is
  # read-only.  Silence wine's debug channels and disable the Mono/Gecko
  # auto-install prompts (they would try to reach the network, which the sandbox
  # forbids) so a bare `--help` run is all that happens.
  export HOME="$TMPDIR"
  export WINEPREFIX="$TMPDIR/wineprefix"
  export WINEDEBUG=-all
  export WINEDLLOVERRIDES="mscoree=d;mshtml=d"
  # Initialize the prefix once up front so the per-binary runs below fail only
  # on the binary, not on first-run prefix setup.
  wineboot --init >/dev/null 2>&1 || true
  status=0
  for pkg in ${
    pkgs.lib.concatStringsSep " "
    (map toString (pkgs.lib.attrValues windowsPackages))
  }; do
    for exe in "$pkg"/bin/*.exe; do
      echo "running $(basename "$exe") --help under wine..." >&2
      if ! wine64 "$exe" --help >/dev/null 2>&1; then
        echo "ERROR: $exe did not execute under wine (--help exited nonzero)." >&2
        echo "The x86_64 Windows cross binary links but does not run." >&2
        status=1
      fi
    done
  done
  test "$status" -eq 0
  touch "$out"
''
