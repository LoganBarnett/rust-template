# mkWindowsMsvcCrossPackages — the opt-in MSVC-ABI Windows variants of a
# workspace's binaries, cross-compiled with the LLVM MSVC-compatible toolchain
# (clang-cl / lld-link / llvm-lib) against an xwin-splatted Microsoft SDK.
#
# The DEFAULT Windows path is gnullvm (mkWindowsCrossPackages): it links no
# Microsoft code, needs no SDK, and is always on.  This helper exists only for
# the rare dependency that requires the MSVC ABI (a prebuilt MSVC-only library,
# MSVC-specific intrinsics), so it is strictly opt-in and produces nothing
# unless a caller passes `xwinSdk`.  It targets `x86_64-pc-windows-msvc` and,
# like the gnullvm helper, is host-agnostic and needs no code signing.
#
# It is x86_64-only on purpose.  `aarch64-pc-windows-msvc` is a Tier-1 target,
# but the `cc` crate drives that triple through clang's GNU driver rather than
# clang-cl, which rejects the MSVC `/imsvc` include flags and `-fPIC`, so any
# dependency that compiles C (a TLS stack, `ring`, a `*-sys` crate) fails to
# build.  arm64 Windows is fully served by the always-on gnullvm default, so
# there is no gap — MSVC is the narrow escape hatch for x86_64 MSVC-ABI deps.
#
# It follows crane's proven offline route: rather than invoking `cargo xwin`
# (which would download the SDK at build time — impossible in the sandbox), it
# sets the linker, compiler, and archiver env directly and points them at the
# pre-fetched SDK, so the ordinary mkRustPackages/crane build runs unchanged.
#
# `xwinSdk` is the splatted SDK derivation (`foundation.lib.xwinSdk { inherit
# pkgs; }`), or null to skip the MSVC build.  Passing it is the opt-in — and
# because that derivation accepts Microsoft's SDK licence at its fetch step,
# evaluating it in the consumer's own flake is where that consent lives,
# mirroring how mkDarwinCrossPackages' `appleSdk` surfaces the Apple SDK licence
# in the consumer's flake rather than hiding it in the foundation library.
#
# Returns a single-system attrset of `<key>-x86_64-windows-msvc` packages
# (empty when `xwinSdk` is null).
{
  # Flake `self` — forwarded to mkRustPackages for `cleanCargoSource` and
  # per-crate override resolution.
  self,
  # Per-system nixpkgs with the rust-overlay applied (for `rust-bin`) and the
  # LLVM MSVC toolchain.
  pkgs,
  # Accepted for call-site symmetry with the other mk*CrossPackages helpers, but
  # unused: the MSVC cross build is driven by the LLVM toolchain, not the Nix
  # `system` being targeted, so this helper is host-agnostic.
  system ? null,
  # Workspace crate map, the same value passed to mkRustPackages.
  crates,
  # The crane flake input, used to build an MSVC-targeted crane lib.
  crane,
  # The xwin-splatted MSVC SDK derivation, or null to skip the MSVC build.  See
  # the header: passing it is the opt-in and the point of licence consent.
  xwinSdk ? null,
  # The caller's crane commonArgs, threaded through so a project's native
  # dependencies reach the MSVC build; the target-specifics below are overlaid.
  commonArgs ? {},
}:
if xwinSdk == null
then {}
else let
  lib = pkgs.lib;
  mkRustPackages = import ./mkRustPackages.nix;
  # clang-cl, lld-link, and llvm-lib — the LLVM stand-ins for cl.exe / link.exe
  # / lib.exe that make an MSVC-target cross-compile work off a Windows host.
  msvcTools = [pkgs.llvmPackages.clang-unwrapped pkgs.lld pkgs.llvm];
  # The SDK's four system-include roots.  They are handed to clang-cl via /imsvc
  # (system includes) rather than /I so clang-cl's own builtin resource headers
  # keep priority — the ordering that otherwise produces a stdalign.h clash.
  includeDirs = [
    "${xwinSdk}/crt/include"
    "${xwinSdk}/sdk/include/ucrt"
    "${xwinSdk}/sdk/include/um"
    "${xwinSdk}/sdk/include/shared"
  ];
  imsvcFlags = lib.concatMapStringsSep " " (d: "/imsvc${d}") includeDirs;
  # The suffix is kept distinct from the gnullvm `-x86_64-windows` output so
  # both can coexist when a project ships each.  x86_64 only — see the header.
  msvcTargets = {
    x86_64-windows-msvc = "x86_64-pc-windows-msvc";
  };
in
  lib.foldlAttrs (
    acc: suffix: target: let
      arch = lib.head (lib.splitString "-" target);
      ccEnvTarget = lib.replaceStrings ["-"] ["_"] target;
      upperTarget = lib.toUpper ccEnvTarget;
      # The SDK's per-arch library roots (filenames lowercased by xwin-sdk.nix
      # so the exact-case lookup resolves on a case-sensitive host).
      libDirs = [
        "${xwinSdk}/crt/lib/${arch}"
        "${xwinSdk}/sdk/lib/um/${arch}"
        "${xwinSdk}/sdk/lib/ucrt/${arch}"
      ];
      libFlags = lib.concatMapStringsSep " " (d: "-Lnative=${d}") libDirs;
      craneLib =
        (crane.mkLib pkgs).overrideToolchain
        (p: p.rust-bin.stable.latest.default.override {targets = [target];});
      msvcArgs =
        commonArgs
        // {
          src = craneLib.cleanCargoSource self;
          CARGO_BUILD_TARGET = target;
          # lld-link is the link.exe-compatible linker rustc drives for an MSVC
          # target; the SDK library roots reach the link through per-target
          # RUSTFLAGS (rustc forwards -Lnative as /LIBPATH:).
          "CARGO_TARGET_${upperTarget}_LINKER" = "lld-link";
          "CARGO_TARGET_${upperTarget}_RUSTFLAGS" = libFlags;
          # clang-cl compiles any cc-crate C/C++ dependency (e.g. ring) and
          # archives with llvm-lib; it needs the target triple and the SDK
          # system-includes.  This mirrors how the gnullvm and darwin helpers
          # route CC/CXX so a C dependency builds in crane's deps-only phase.
          "CC_${ccEnvTarget}" = "clang-cl";
          "CXX_${ccEnvTarget}" = "clang-cl";
          "AR_${ccEnvTarget}" = "llvm-lib";
          "CFLAGS_${ccEnvTarget}" = "--target=${target} ${imsvcFlags}";
          "CXXFLAGS_${ccEnvTarget}" = "--target=${target} ${imsvcFlags}";
          # A PE cannot run on the build host, so the test phase is skipped —
          # the same sources are gated by the native build and the workspace
          # test check in the same release.  Nothing to re-sign for Windows.
          doCheck = false;
          nativeBuildInputs =
            (commonArgs.nativeBuildInputs or [])
            ++ msvcTools;
        };
      msvcPackages =
        (mkRustPackages {
          inherit self pkgs crates;
          craneLib = craneLib;
          commonArgs = msvcArgs;
        }).packages;
    in
      acc
      // lib.mapAttrs' (
        key: pkg: lib.nameValuePair "${key}-${suffix}" pkg
      )
      msvcPackages
  ) {}
  msvcTargets
