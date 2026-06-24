# mkDarwinCrossPackages — macOS variants of a workspace's binaries, cross-
# compiled on an x86_64-linux builder so a release needs no macOS runner.
#
# GitHub's hosted macOS runners are scarce and slow (a free-tier darwin build
# can sit queued for hours), while Linux runners are abundant.  rustc already
# emits `*-apple-darwin` object code on Linux; the only blocker is the linker —
# nixpkgs' darwin cross stdenv is gated because Apple's `cctools`/`ld64` is
# darwin-only.  zig ships its own Mach-O linker that runs on Linux, so this
# helper drives the build with `cargo zigbuild`, sidestepping `cctools`
# entirely.  The linked binaries even carry an ad-hoc (linker-signed) signature,
# which is what lets an arm64 Mach-O execute at all — no separate `codesign`.
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
#     // mkMuslPackages {inherit self pkgs system crates crane;}
#     // mkDarwinCrossPackages {inherit self pkgs system crates crane;}
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
  # The crane flake input, used only for `vendorCargoDeps` (offline vendoring of
  # the locked dependency graph, including any git dependencies).
  crane,
  # The Apple SDK interface as a derivation (e.g. `pkgs.apple-sdk.src`), or null
  # for libSystem-only executables that link no Apple frameworks.
  appleSdk ? null,
}: let
  lib = pkgs.lib;
  # The darwin targets cross-built from x86_64-linux, keyed by the package-name
  # suffix (which matches the Nix darwin system names).
  darwinTargets = {
    aarch64-darwin = "aarch64-apple-darwin";
    x86_64-darwin = "x86_64-apple-darwin";
  };
in
  if system != "x86_64-linux"
  then {}
  else let
    # Vendor the workspace's locked deps once (shared across crates and both
    # targets) so each build runs fully offline in the sandbox.
    cargoVendorDir = (crane.mkLib pkgs).vendorCargoDeps {src = self;};

    # cargo's per-target RUSTFLAGS env var, e.g.
    # CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS.
    rustflagsEnvVar = target:
      "CARGO_TARGET_"
      + lib.toUpper (lib.replaceStrings ["-"] ["_"] target)
      + "_RUSTFLAGS";

    frameworks = "${toString appleSdk}/System/Library/Frameworks";

    # One crate, one darwin target.
    buildCrate = suffix: target: key: crate: let
      rust = pkgs.rust-bin.stable.latest.default.override {
        targets = [target];
      };
    in
      pkgs.stdenv.mkDerivation {
        name = "${crate.name}-${suffix}";
        src = self;
        # rust supplies the darwin-target std; cargo-zigbuild + zig are the
        # Mach-O cross-linker that replaces the darwin-gated cctools.  stdenv
        # (not stdenvNoCC): proc-macro crates link as host x86_64-linux
        # artifacts via the stdenv cc; only the darwin target link uses zig.
        # libclang is only needed by crates whose build scripts run bindgen,
        # all of which are framework crates, so it rides the appleSdk path.
        nativeBuildInputs =
          [rust pkgs.cargo-zigbuild pkgs.zig]
          ++ lib.optional (appleSdk != null) pkgs.libclang;
        buildPhase = ''
          runHook preBuild
          export HOME="$TMPDIR"
          export CARGO_HOME="$TMPDIR/cargo"
          export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-cache"
          mkdir --parents "$CARGO_HOME"
          cp ${cargoVendorDir}/config.toml "$CARGO_HOME/config.toml"
          export CARGO_NET_OFFLINE=true
          ${lib.optionalString (appleSdk != null) ''
            # The inert SDK supplies framework headers (bindgen) and .tbd stubs
            # (linker).  zig's Mach-O linker does not derive the framework
            # search path from -isysroot, so hand it -F/-L explicitly, target-
            # scoped so host proc-macro links keep the native linker.
            export SDKROOT="${toString appleSdk}"
            export LIBCLANG_PATH="${pkgs.libclang.lib}/lib"
            export BINDGEN_EXTRA_CLANG_ARGS="-isysroot ${toString appleSdk} -target ${target} -F ${frameworks}"
            export ${rustflagsEnvVar target}="-Clink-arg=-F${frameworks} -Clink-arg=-L${toString appleSdk}/usr/lib"
          ''}
          cargo zigbuild --release --offline \
            --package ${crate.name} --target ${target}
          runHook postBuild
        '';
        installPhase = ''
          runHook preInstall
          mkdir --parents "$out/bin"
          cp "target/${target}/release/${crate.binary}" "$out/bin/${crate.binary}"
          runHook postInstall
        '';
        # Built artifacts are Mach-O; the host can neither run nor strip them.
        dontStrip = true;
        doCheck = false;
      };
  in
    lib.foldlAttrs (
      acc: suffix: target:
        acc
        // lib.mapAttrs' (
          key: crate:
            lib.nameValuePair "${key}-${suffix}" (buildCrate suffix target key crate)
        )
        crates
    ) {}
    darwinTargets
