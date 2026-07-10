# llvm-mingw — mstorsjo's prebuilt LLVM/Clang mingw-w64 cross toolchain,
# vendored for the Nix build host so the `*-pc-windows-gnullvm` targets can
# link (and compile C/C++) inside the sandbox without Docker.
#
# rustc's gnullvm Windows targets need an LLVM-based mingw C toolchain — clang +
# lld + LLVM's compiler-rt/libunwind/libc++ plus the mingw-w64 CRT headers,
# startup objects, and import libraries.  Unlike GCC-mingw (x86-only, and no
# Windows-on-ARM), one llvm-mingw tree covers every Windows architecture, which
# is exactly what lets a single toolchain serve both the x86_64 and aarch64
# Windows builds.  cargo-zigbuild — the linker the darwin/gnu-portable helpers
# use — explicitly supports only Linux and macOS targets, so the zig path does
# not extend to Windows; llvm-mingw is the toolchain instead.
#
# llvm-mingw is not packaged in nixpkgs, so this fetches the upstream release
# for the *build host* as a fixed-output derivation (a pinned-hash fetch is the
# sanctioned way to pull a non-nixpkgs artifact into a sandboxed build).  The
# Linux release ships ELF binaries built for a generic distro, so
# autoPatchelfHook rewrites their interpreter and RPATH onto the Nix stdenv; the
# macOS release is a universal Mach-O that runs natively and is used as-is.  All
# variants are the UCRT build, matching the gnullvm targets' Universal CRT
# runtime.
#
# Only the four host platforms below are covered (Linux and macOS, the CI
# runners and a contributor's Mac); the helper errors on any other build host
# rather than silently producing a broken toolchain.  Bump `version` and the
# four hashes together when updating — one upstream tag ships all hosts.
#
# Returns a derivation whose `bin/` holds the per-arch wrappers
# (`<arch>-w64-mingw32-clang`, `-clang++`, `-ar`, …) and `lld`.
{pkgs}: let
  lib = pkgs.lib;
  version = "20260616";
  base = "https://github.com/mstorsjo/llvm-mingw/releases/download/${version}";
  # The upstream release asset per build host, with its fixed-output hash.  The
  # macOS asset is a universal binary, so one entry serves both Apple
  # architectures.
  hostRelease = {
    x86_64-linux = {
      file = "llvm-mingw-${version}-ucrt-ubuntu-22.04-x86_64.tar.xz";
      hash = "sha256-U0uS4GeyKmtEQfSK6SQKM0GxeCXQTVd+qwz4XES03to=";
    };
    aarch64-linux = {
      file = "llvm-mingw-${version}-ucrt-ubuntu-22.04-aarch64.tar.xz";
      hash = "sha256-5+XRNdk9Pyo76q6mM6Ww5mrHU5GlP+rmVDkZE912ECs=";
    };
    x86_64-darwin = {
      file = "llvm-mingw-${version}-ucrt-macos-universal.tar.xz";
      hash = "sha256-LKsCoulkvUqumBFQpFmF0HxlfPqNJElZ654tzF7t17E=";
    };
    aarch64-darwin = {
      file = "llvm-mingw-${version}-ucrt-macos-universal.tar.xz";
      hash = "sha256-LKsCoulkvUqumBFQpFmF0HxlfPqNJElZ654tzF7t17E=";
    };
  };
  system = pkgs.stdenv.hostPlatform.system;
  release =
    hostRelease.${system}
    or (throw "llvm-mingw: unsupported build host '${system}'");
  isLinux = pkgs.stdenv.hostPlatform.isLinux;
in
  pkgs.stdenvNoCC.mkDerivation {
    pname = "llvm-mingw";
    inherit version;
    src = pkgs.fetchurl {
      url = "${base}/${release.file}";
      inherit (release) hash;
    };
    # The Linux release's ELF tools carry a generic-distro interpreter and
    # RPATH; autoPatchelfHook repoints them at the Nix stdenv.  The macOS
    # universal Mach-O runs natively, so it needs neither the hook nor the libs.
    nativeBuildInputs = lib.optionals isLinux [pkgs.autoPatchelfHook];
    buildInputs = lib.optionals isLinux [
      pkgs.stdenv.cc.cc.lib
      pkgs.zlib
      pkgs.libxml2
      pkgs.ncurses
    ];
    # The wrappers locate their sibling tools and the CRT/headers by paths
    # relative to their own location, so the extracted tree is kept intact under
    # $out rather than picked apart.  unpackPhase cd's into the single top-level
    # release directory, so `.` here is that tree.
    installPhase = ''
      runHook preInstall
      mkdir --parents "$out"
      cp --archive . "$out"
      runHook postInstall
    '';
    # A prebuilt toolchain we only unpack — nothing to strip, and stripping the
    # cross-compiler's own binaries would only risk breaking them.
    dontStrip = true;
  }
