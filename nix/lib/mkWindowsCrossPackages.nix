# mkWindowsCrossPackages — native Windows (PE) variants of a workspace's
# binaries, cross-compiled via llvm-mingw for the gnullvm targets.
#
# This is the Windows sibling of mkDarwinCrossPackages / mkGnuPortablePackages:
# it runs through mkRustPackages/crane exactly the same way, threading the
# caller's whole commonArgs, only with a Windows target and llvm-mingw swapped
# in for the linker and C toolchain.  The produced binaries are ordinary
# Windows PE executables — they use the Win32 API directly, so they run on a
# stock Windows install with no Cygwin/MSYS2 POSIX layer.  The compiler runtime
# (the LLVM unwinder and the CRT) is linked statically — see the crt-static
# rustflag below — so for pure-Rust code they import only always-present Win32
# system DLLs and carry no dependency on a redistributable runtime.
#
# Why gnullvm and not the classic `*-windows-gnu` (mingw64): GCC-mingw has no
# Windows-on-ARM, and cargo-zigbuild — the linker the darwin/gnu-portable
# helpers use — supports only Linux and macOS targets, so neither the zig path
# nor GCC-mingw covers aarch64 Windows.  The `*-pc-windows-gnullvm` targets
# (Tier 2 with host tools since Rust 1.91) use LLVM's mingw runtime instead of
# GCC's, one toolchain (llvm-mingw) covers both architectures, and rustup ships
# prebuilt std for each — so this helper needs a pinned toolchain ≥ 1.91.
#
# Unlike mkDarwinCrossPackages (gated to x86_64-linux because the Apple SDK
# dependency breaks elsewhere and a Mac builds darwin natively), this helper is
# host-agnostic: llvm-mingw ships a per-host toolchain (see llvm-mingw.nix), so
# the Windows cross build runs on any supported build host — a Linux CI runner
# or a contributor's Mac alike — and there is no `system` gate.  There is also
# no signing step: Windows loads unsigned PEs, so none of the ad-hoc-signature
# dance mkDarwinCrossPackages needs for arm64 Mach-O applies here.
#
# Usage (mirrors the other cross helpers):
#
#   packages =
#     rustPackages.packages
#     // mkMuslPackages {inherit self pkgs system crates crane commonArgs;}
#     // mkWindowsCrossPackages {
#       inherit self pkgs system crates crane commonArgs;
#     }
#     // {default = ...;};
#
# Returns a single-system attrset of `<key>-x86_64-windows` and
# `<key>-aarch64-windows` packages.
{
  # Flake `self` — forwarded to mkRustPackages for `cleanCargoSource` and
  # per-crate override resolution.
  self,
  # Per-system nixpkgs with the rust-overlay applied (for `rust-bin`).  Its
  # host platform selects the llvm-mingw release; see llvm-mingw.nix.
  pkgs,
  # Accepted for call-site symmetry with the other mk*Packages helpers, but
  # unused: the Windows cross build is driven by the *build host* (llvm-mingw
  # ships a per-host toolchain), not by the Nix `system` being targeted, so
  # this helper is host-agnostic and never returns an empty set on that basis.
  system ? null,
  # Workspace crate map, the same value passed to mkRustPackages.
  crates,
  # The crane flake input, used to build a Windows-targeted crane lib.
  crane,
  # The caller's crane commonArgs (buildInputs, nativeBuildInputs, env, …),
  # threaded through so a project's native dependencies reach the Windows build
  # the same way they reach mkRustPackages; the target-specifics below are
  # overlaid on top.  Defaults to empty for callers that pass none.
  commonArgs ? {},
}: let
  lib = pkgs.lib;
  mkRustPackages = import ./mkRustPackages.nix;
  llvmMingw = import ./llvm-mingw.nix {inherit pkgs;};
  # rustc gnullvm target ⇽ the package-name suffix (arch-os, matching the
  # naming the other cross helpers use).
  windowsTargets = {
    x86_64-windows = "x86_64-pc-windows-gnullvm";
    aarch64-windows = "aarch64-pc-windows-gnullvm";
  };
in
  lib.foldlAttrs (
    acc: suffix: target: let
      # The llvm-mingw wrapper prefix for this target's clang/ar, e.g.
      # `${llvmMingw}/bin/aarch64-w64-mingw32`.
      mingwArch = lib.head (lib.splitString "-" target);
      toolPrefix = "${llvmMingw}/bin/${mingwArch}-w64-mingw32";
      # A crane lib carrying this Windows target's std (rustup/rust-overlay ship
      # a prebuilt gnullvm std, so nothing is compiled from source here).  The
      # pinned toolchain must be ≥ 1.91 for the aarch64 gnullvm std to exist.
      craneLib =
        (crane.mkLib pkgs).overrideToolchain
        (p: p.rust-bin.stable.latest.default.override {targets = [target];});
      # cargo's per-target env-var suffix (triple with dashes → underscores).
      ccEnvTarget = lib.replaceStrings ["-"] ["_"] target;
      windowsArgs =
        commonArgs
        // {
          src = craneLib.cleanCargoSource self;
          CARGO_BUILD_TARGET = target;
          # llvm-mingw's clang wrapper is both the linker (it drives lld and
          # supplies the mingw-w64 CRT, startup objects, and import libraries)
          # and the C/C++ compiler for any cc-crate dependency, so every cargo
          # phase — crane's deps-only build included — uses one toolchain.  This
          # mirrors how the darwin/gnu-portable helpers route CC/CXX through
          # zig: a C-compiling dependency (ring, a *-sys crate, …) compiles in
          # the deps-only phase too, where a host-cc fallback would choke on
          # the Windows target.  llvm-ar is used for AR: it is format-agnostic
          # and always present, avoiding any per-arch ar-name guessing.
          "CARGO_TARGET_${lib.toUpper ccEnvTarget}_LINKER" = "${toolPrefix}-clang";
          "CC_${ccEnvTarget}" = "${toolPrefix}-clang";
          "CXX_${ccEnvTarget}" = "${toolPrefix}-clang++";
          "AR_${ccEnvTarget}" = "${llvmMingw}/bin/llvm-ar";
          # Statically link the compiler runtime so the PE is self-contained.
          # llvm-mingw links its LLVM unwinder as a shared library by default,
          # so a bare gnullvm binary imports libunwind.dll — not a Windows
          # system DLL — and fails to start with STATUS_DLL_NOT_FOUND on a stock
          # install (and under the wine smoke check).  Rust's crt-static target
          # feature links the unwinder and the CRT statically, so the PE imports
          # only always-present Win32 system DLLs.  A plain
          # `-C link-arg=-static` does not suffice for the Rust link — it
          # leaves the libunwind.dll import in place, so crt-static is the knob
          # that removes it.  This is a target-scoped rustflag
          # (CARGO_TARGET_<triple>_RUSTFLAGS), so it governs only the Windows
          # artifacts, never the host build scripts and proc-macros cargo
          # compiles for the build platform, and it is set identically for
          # crane's deps-only and final builds (both read windowsArgs) so the
          # two stay cache-consistent.
          "CARGO_TARGET_${lib.toUpper ccEnvTarget}_RUSTFLAGS" = "-C target-feature=+crt-static";
          # A PE cannot run on the build host, so the test phase is skipped —
          # the same sources are gated by the native build and the workspace
          # test check in the same release.  rustc's own `strip = true` still
          # runs and is PE-aware, and there is nothing to re-sign.
          doCheck = false;
          # The clang wrappers invoke sibling tools (lld, the CRT) by paths
          # relative to their own location, but a build script that scrubs PATH
          # could still break them, so llvm-mingw is put on PATH as well as
          # referenced absolutely above.
          nativeBuildInputs =
            (commonArgs.nativeBuildInputs or [])
            ++ [llvmMingw];
        };
      windowsPackages =
        (mkRustPackages {
          inherit self pkgs crates;
          craneLib = craneLib;
          commonArgs = windowsArgs;
        }).packages;
    in
      acc
      // lib.mapAttrs' (
        key: pkg: lib.nameValuePair "${key}-${suffix}" pkg
      )
      windowsPackages
  ) {}
  windowsTargets
