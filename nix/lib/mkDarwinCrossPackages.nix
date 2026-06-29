# mkDarwinCrossPackages — macOS variants of a workspace's binaries, cross-
# compiled on an x86_64-linux builder so a release needs no macOS runner.
#
# GitHub's hosted macOS runners are scarce and slow (a free-tier darwin build
# can sit queued for hours), while Linux runners are abundant.  rustc already
# emits `*-apple-darwin` object code on Linux; the only blocker is the linker —
# nixpkgs' darwin cross stdenv is gated because Apple's `cctools`/`ld64` is
# darwin-only.  zig ships its own Mach-O linker that runs on Linux, so this
# helper drives crane's build with `cargo zigbuild`, sidestepping `cctools`
# entirely.  The linked binaries even carry an ad-hoc (linker-signed) signature,
# which is what lets an arm64 Mach-O execute at all — no separate `codesign`.
#
# It runs through mkRustPackages/crane exactly like mkMuslPackages, only with a
# darwin target and the zig build command swapped in.  That means the caller's
# whole commonArgs is threaded — buildInputs, nativeBuildInputs, env, every
# crane knob — so a darwin crate gets its native dependencies the same way a
# musl crate does, and crane handles vendoring, the shared deps-only build, and
# installing the binary from the target subdirectory.
#
# Only an x86_64-linux build platform is supported.  aarch64-linux cannot build
# the Apple SDK (a dependency breaks), and a native darwin host builds natively;
# for any other system the helper returns an empty set so a caller can merge its
# result unconditionally — exactly like mkMuslPackages.
#
# libSystem-only executables need no SDK: zig bundles the libSystem stubs, and
# the result is a clean, licence-free build.  An executable that links Apple
# frameworks (anything pulling cpal, objc2-*, core-foundation, security-
# framework, coreaudio-sys, …) needs Apple's framework headers and `.tbd` link
# stubs.  Pass that interface via `appleSdk` — the `apple-sdk.src` derivation, a
# plain fetch of `MacOSX<ver>.sdk` (the full `apple-sdk` package does not build
# on Linux, but its `.src` is inert data and does).  `appleSdk` is the SDK
# derivation itself, not a flag: presence is the signal, and it leaves room to
# pass a pinned or non-nixpkgs SDK later.  Because `apple-sdk` is unfree and
# darwin-gated, the caller's nixpkgs must set `config.allowUnfree` and
# `config.allowUnsupportedSystem` to evaluate `pkgs.apple-sdk.src` — keeping the
# licence acceptance visible in the consumer's own flake.
#
# Usage (mirrors mkMuslPackages):
#
#   packages =
#     rustPackages.packages
#     // mkMuslPackages {inherit self pkgs system crates crane commonArgs;}
#     // mkDarwinCrossPackages {
#       inherit self pkgs system crates crane commonArgs;
#     }
#     // {default = ...;};
#
# For a framework-linking workspace, add `appleSdk = pkgs.apple-sdk.src;`.
#
# Returns a single-system attrset of `<key>-aarch64-darwin` and
# `<key>-x86_64-darwin` packages (empty unless building on x86_64-linux).
{
  # Flake `self` — the workspace source, vendored and built.
  self,
  # Per-system nixpkgs with the rust-overlay applied (for `rust-bin`).  Only the
  # x86_64-linux instance produces output.
  pkgs,
  # The Nix system being built for; output is empty unless x86_64-linux.
  system,
  # Workspace crate map, the same value passed to mkRustPackages.
  crates,
  # The crane flake input, used to build a darwin-targeted crane lib.
  crane,
  # The Apple SDK interface as a derivation (e.g. `pkgs.apple-sdk.src`), or null
  # for libSystem-only executables that link no Apple frameworks.
  appleSdk ? null,
  # The caller's crane commonArgs (such as buildInputs, nativeBuildInputs, or
  # env), threaded through so a project's native dependencies reach the darwin
  # build the same way they reach mkRustPackages; the cross target-specifics
  # below are overlaid on top.  Defaults to empty for callers that pass none.
  commonArgs ? {},
}: let
  lib = pkgs.lib;
  mkRustPackages = import ./mkRustPackages.nix;
  # The darwin targets cross-built from x86_64-linux, keyed by the package-name
  # suffix (which matches the Nix darwin system names).
  darwinTargets = {
    aarch64-darwin = "aarch64-apple-darwin";
    x86_64-darwin = "x86_64-apple-darwin";
  };
  # cargo's per-target RUSTFLAGS env var, e.g.
  # CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS.
  rustflagsEnvVar = target:
    "CARGO_TARGET_"
    + lib.toUpper (lib.replaceStrings ["-"] ["_"] target)
    + "_RUSTFLAGS";
in
  if system != "x86_64-linux"
  then {}
  else
    lib.foldlAttrs (
      acc: suffix: target: let
        # A crane lib carrying the darwin target's std.  cargo-zigbuild + zig
        # below are the Mach-O cross-linker that replaces the darwin-gated
        # cctools; libclang is only needed by crates whose build scripts run
        # bindgen, all of which are framework crates, so it rides the appleSdk
        # path.
        craneLib =
          (crane.mkLib pkgs).overrideToolchain
          (p: p.rust-bin.stable.latest.default.override {targets = [target];});
        frameworks = "${toString appleSdk}/System/Library/Frameworks";
        # Framework crates need the SDK's headers (bindgen) and `.tbd` stubs
        # (linker); a libSystem-only crate needs none of this.  zig's Mach-O
        # linker does not derive the framework search path from -isysroot, so it
        # is handed -F/-L explicitly, target-scoped so host proc-macro links
        # keep the native linker.
        sdkEnv = lib.optionalAttrs (appleSdk != null) {
          SDKROOT = toString appleSdk;
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          BINDGEN_EXTRA_CLANG_ARGS =
            "-isysroot ${toString appleSdk}"
            + " -target ${target}"
            + " -F ${frameworks}";
          ${rustflagsEnvVar target} =
            "-Clink-arg=-F${frameworks}"
            + " -Clink-arg=-L${toString appleSdk}/usr/lib";
        };
        darwinArgs =
          commonArgs
          // sdkEnv
          // {
            src = craneLib.cleanCargoSource self;
            CARGO_BUILD_TARGET = target;
            # zig is the linker; cargo-zigbuild drives it for the final link.
            cargoBuildCommand = "cargo zigbuild --release";
            # Built artifacts are Mach-O: the x86_64-linux host can neither run
            # the tests nor strip the binary.
            doCheck = false;
            dontStrip = true;
            nativeBuildInputs =
              (commonArgs.nativeBuildInputs or [])
              ++ [pkgs.cargo-zigbuild pkgs.zig]
              ++ lib.optional (appleSdk != null) pkgs.libclang;
            # cargo-zigbuild caches under $HOME/.cache and zig under its own
            # cache dir; crane's HOME=/homeless-shelter is read-only, so point
            # both at the writable build tree.
            preBuild =
              (commonArgs.preBuild or "")
              + ''
                export HOME="$TMPDIR"
                export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-cache"
              '';
          };
        darwinPackages =
          (mkRustPackages {
            inherit self pkgs crates;
            craneLib = craneLib;
            commonArgs = darwinArgs;
          }).packages;
      in
        acc
        // lib.mapAttrs' (
          key: pkg: lib.nameValuePair "${key}-${suffix}" pkg
        )
        darwinPackages
    ) {}
    darwinTargets
