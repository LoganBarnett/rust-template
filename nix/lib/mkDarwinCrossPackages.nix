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
        # The C/C++ cross toolchain for this darwin target.  A dependency whose
        # build script compiles C or assembly through the `cc` crate (`ring`,
        # say, pulled in by a TLS stack) does so in every cargo phase —
        # including crane's deps-only `cargo check`, not just the final
        # `cargo zigbuild`.  cargo-zigbuild sets the cc-crate cross vars only
        # for its own invocation, so those other phases fall back to the host
        # `gcc`, which chokes on the Apple flags (`-arch`,
        # `-mmacosx-version-min`, …) the `cc` crate emits for a `*-apple-darwin`
        # target.  Pointing CC/CXX at the zig-cc wrappers here makes the C
        # toolchain identical in every phase.  These mirror the wrappers
        # cargo-zigbuild writes internally — same
        # `zig cc`/`zig c++` shim, same args — and it honours ours because it
        # only sets its own when the var is unset.  zig is forced onto PATH so
        # the wrapper works even under a build script that scrubs it.
        zigArch = lib.head (lib.splitString "-" target);
        ccEnvTarget = lib.replaceStrings ["-"] ["_"] target;
        zigCcArgs = "-g -fno-sanitize=all -target ${zigArch}-macos-none";
        zigCc = pkgs.writeShellScript "zigcc-${target}" ''
          export PATH="${pkgs.zig}/bin:$PATH"
          exec ${pkgs.cargo-zigbuild}/bin/cargo-zigbuild zig cc \
            -- ${zigCcArgs} "$@"
        '';
        zigCxx = pkgs.writeShellScript "zigcxx-${target}" ''
          export PATH="${pkgs.zig}/bin:$PATH"
          exec ${pkgs.cargo-zigbuild}/bin/cargo-zigbuild zig c++ \
            -- ${zigCcArgs} "$@"
        '';
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
            # See the zigCc/zigCxx comment above: these route the C
            # compilation of every cargo phase through zig, not just the final
            # zigbuild.  The env var name is the cc-crate's underscored triple.
            "CC_${ccEnvTarget}" = "${zigCc}";
            "CXX_${ccEnvTarget}" = "${zigCxx}";
            # zig is the linker; cargo-zigbuild drives it for the final link.
            cargoBuildCommand = "cargo zigbuild --release";
            # Built artifacts are Mach-O: the x86_64-linux host can neither run
            # the tests nor strip the binary.
            doCheck = false;
            dontStrip = true;
            nativeBuildInputs =
              (commonArgs.nativeBuildInputs or [])
              ++ [pkgs.cargo-zigbuild pkgs.zig]
              # rcodesign re-signs the arm64 binary after cargo's strip — see
              # the postFixup below.  Only arm64 needs it, so it is left out of
              # the x86_64 build.
              ++ lib.optional (zigArch == "aarch64") pkgs.rcodesign
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
            # Re-sign every installed arm64 binary ad-hoc as the final build
            # step.  zig stamps an ad-hoc (linker-signed) Mach-O signature
            # during the link — the thing that lets an arm64 Mach-O execute at
            # all — but the release profile's `strip = true` runs after that
            # link and rewrites the binary, leaving a signature that no longer
            # matches its bytes.  (`dontStrip` above only disables nixpkgs' own
            # strip hook, not cargo's.)  An arm64 Mach-O with an invalid
            # signature is SIGKILLed by the kernel on load — silently, with no
            # output — so the signature must be re-applied once nothing else
            # will touch the binary.  rcodesign is the only Mach-O signer that
            # runs on this Linux builder; `sign` with no signing key produces
            # an ad-hoc signature and rewrites the binary in place.  Uses
            # postFixup so it follows the fixup phase, the last thing that
            # could mutate the binary.
            #
            # Only arm64 is re-signed.  x86_64 macOS does not enforce code
            # signatures, so zig leaves that binary unsigned and reserves no
            # Mach-O header room for a signature load command — rcodesign there
            # fails with "insufficient room to write code signature load
            # command", and an unsigned x86_64 binary runs fine regardless.
            postFixup =
              (commonArgs.postFixup or "")
              + lib.optionalString (zigArch == "aarch64") ''
                for macho in "$out"/bin/*; do
                  test -f "$macho" || continue
                  chmod +w "$macho"
                  rcodesign sign "$macho"
                done
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
