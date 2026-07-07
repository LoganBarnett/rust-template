# mkDarwinSignatureCheck — a `nix flake check` derivation that guards the
# ad-hoc Mach-O signatures on arm64 darwin cross binaries against regression.
#
# mkDarwinCrossPackages re-signs each arm64 darwin binary ad-hoc as its final
# build step: zig stamps a linker-signed signature during the cross link, but
# the release profile's `strip = true` runs afterward and rewrites the binary,
# leaving a signature that no longer matches its bytes.  An arm64 Mach-O with
# an invalid signature is SIGKILLed by the kernel on load — silently, with no
# output — so the re-sign is load-bearing, and a regression that drops or
# undoes it ships a binary that dies on Apple Silicon with no diagnostic.  Only
# arm64 is re-signed (x86_64 macOS does not enforce signatures), so callers
# pass only the `-aarch64-darwin` packages; an unsigned x86_64 binary would
# make the re-sign-and-compare below fail spuriously.
#
# No tool on the Linux builder cryptographically verifies an ad-hoc signature:
# Apple's `codesign` is macOS-only, and rcodesign's `verify` rejects ad-hoc
# signatures outright (it expects a CMS/certificate blob that ad-hoc signing
# does not carry).  rcodesign *signing* is deterministic, though, so re-signing
# a copy and byte-comparing is a faithful proxy — an intact final signature
# reproduces exactly, while a binary stripped or otherwise mutated after
# signing (or never re-signed) does not.
#
# `darwinPackages` is an attrset of the arm64 darwin cross derivations to
# verify (the `-aarch64-darwin`-suffixed subset of a flake's packages).  It is
# empty on every system but x86_64-linux, so the caller gates the check with
# `lib.optionalAttrs (darwinPackages != {})`.
{
  # A per-system nixpkgs, used for `rcodesign` and `runCommand`.
  pkgs,
  # The arm64 darwin cross packages to verify, keyed however the caller likes;
  # only the values (derivations exposing `$out/bin/<binary>`) are used.
  darwinPackages,
}:
pkgs.runCommand "darwin-signatures-adhoc"
{nativeBuildInputs = [pkgs.rcodesign];}
''
  set -euo pipefail
  shopt -s nullglob
  status=0
  for pkg in ${
    pkgs.lib.concatStringsSep " "
    (map toString (pkgs.lib.attrValues darwinPackages))
  }; do
    for macho in "$pkg"/bin/*; do
      cp "$macho" candidate
      chmod +w candidate
      rcodesign sign candidate >/dev/null 2>&1
      if ! cmp --silent "$macho" candidate; then
        echo "ERROR: $macho is not idempotently ad-hoc signed." >&2
        echo "It was modified after signing (e.g. stripped) or never" \
          "re-signed by mkDarwinCrossPackages; Apple Silicon SIGKILLs" \
          "such a binary with no output." >&2
        status=1
      fi
      rm --force candidate
    done
  done
  test "$status" -eq 0
  touch "$out"
''
