{
  description = "Rust Template - Best-in-class Rust project setup";
  inputs = {
    # LLM: Do NOT change this URL unless explicitly directed. This is the
    # correct format for nixpkgs stable (25.11 is correct, not nixos-25.11).
    nixpkgs.url = "github:NixOS/nixpkgs/25.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    changelog-roller.url = "github:LoganBarnett/changelog-roller";
    foundation.url = "github:LoganBarnett/rust-template";
    foundation.inputs.nixpkgs.follows = "nixpkgs";
    org-fmt.url = "github:LoganBarnett/org-fmt";
    org-fmt.inputs.nixpkgs.follows = "nixpkgs";
    org-fmt.inputs.rust-overlay.follows = "rust-overlay";
    org-fmt.inputs.crane.follows = "crane";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    crane,
    changelog-roller,
    foundation,
    org-fmt,
  } @ inputs: let
    forAllSystems =
      nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
    perSystem = forAllSystems (system: let
      # Hoisted so the quarantined `pkgsUnfreeFor` instance below (used only
      # when this project links Apple frameworks) stays overlay-consistent with
      # this build `pkgs` — both see the same overlay set.
      overlays = [(import rust-overlay)];
      pkgs = import nixpkgs {
        inherit system overlays;
      };
      craneLib =
        (crane.mkLib pkgs).overrideToolchain
        (p: p.rust-bin.stable.latest.default);
      rust = pkgs.rust-bin.stable.latest.default.override {
        extensions = [
          # For rust-analyzer and others.  See
          # https://nixos.wiki/wiki/Rust#Shell.nix_example for details.
          "rust-src"
          "rust-analyzer"
          "rustfmt"
        ];
      };
      crates = {
        # CRATE_ENTRIES

        # Note: The 'lib' crate is not included here as it doesn't
        # produce a binary.
      };
      commonArgs = {
        src = craneLib.cleanCargoSource self;
        # This governs only the whole-workspace `default` package below; the
        # per-crate packages and the workspace test check get their test scope
        # from mkRustPackages, which overrides this.  Run only unit tests
        # (--lib --bins) and skip the integration tests under tests/, which may
        # need services unavailable in the Nix sandbox.
        cargoTestExtraArgs = "--lib --bins";
      };
      rustPackages = foundation.lib.mkRustPackages {
        inherit self pkgs craneLib crates commonArgs;
      };
      # On Linux each binary also gets a statically-linked `<name>-musl`
      # variant; on other systems mkMuslPackages returns an empty set.  It
      # threads the same commonArgs, so a project's native dependencies (such as
      # alsa) reach the musl build as they do the native one.
      muslPackages = foundation.lib.mkMuslPackages {
        inherit self pkgs system crates crane commonArgs;
      };
      # On Linux each binary also gets a portable `<name>-gnu` variant: a
      # dynamic glibc build that runs off the Nix store (FHS interpreter, glibc
      # 2.17 floor) and links the host's shared libraries.  It threads the same
      # commonArgs, so a project's native dependencies (such as alsa) reach it
      # the same way — pick this over musl for a tool that must use a host
      # library with a runtime plugin/dlopen ecosystem.  Empty on other systems.
      gnuPortablePackages = foundation.lib.mkGnuPortablePackages {
        inherit self pkgs system crates crane commonArgs;
      };
      # The x86_64-linux build cross-compiles macOS `<key>-<arch>-darwin`
      # variants via zig so a release needs no macOS runner; empty on other
      # systems.  A crate that links Apple frameworks (transitively pulling
      # cpal, objc2-*, security-framework, the auth TLS stack, and similar) also
      # needs the Apple SDK's headers and link stubs.  That is opt-in: set
      # `"apple-frameworks": true` in rust-template.json — the same flag shape
      # as `windows-msvc` below.  When set, `appleSdk` is taken from a
      # quarantined unfree nixpkgs (foundation.lib.pkgsUnfreeFor) that accepts
      # the darwin-gated Apple SDK licence; evaluating it accepts that licence
      # in this project's own flake — the visible consent — while leaving this
      # build `pkgs` graph free.  Left false (the default) no SDK is wired and
      # the cross build stays licence-free.  See CONTRIBUTING.org.
      appleFrameworksEnabled =
        (builtins.fromJSON (builtins.readFile ./rust-template.json)).apple-frameworks
        or false;
      darwinCrossPackages = foundation.lib.mkDarwinCrossPackages {
        inherit self pkgs system crates crane commonArgs;
        appleSdk =
          if appleFrameworksEnabled
          then (foundation.lib.pkgsUnfreeFor {inherit nixpkgs system overlays;}).apple-sdk.src
          else null;
      };
      # Native Windows PE variants (`<key>-{x86_64,aarch64}-windows`),
      # cross-compiled via llvm-mingw for the gnullvm targets — no Microsoft
      # SDK, no Cygwin/MSYS2 runtime; a pure-Rust binary needs only the OS
      # Universal CRT (Windows 10+).  Unlike the darwin cross build this is
      # host-agnostic (llvm-mingw ships a per-host toolchain), so it builds on
      # the Linux CI runners and on a contributor's Mac alike.  Requires a
      # toolchain ≥ Rust 1.91 for the aarch64 gnullvm std — see
      # CONTRIBUTING.org.
      windowsCrossPackages = foundation.lib.mkWindowsCrossPackages {
        inherit self pkgs system crates crane commonArgs;
      };
      # The opt-in MSVC-ABI Windows variant
      # (`<key>-x86_64-windows-msvc`), for a dependency that requires the MSVC
      # ABI rather than the default gnullvm path above.  Off unless
      # `"windows-msvc": true` is set in rust-template.json — that flag hands
      # the helper the xwin-splatted Microsoft SDK (foundation.lib.xwinSdk), and
      # evaluating it accepts Microsoft's SDK licence in this project's own
      # flake: the visible consent, exactly as `appleSdk` surfaces the Apple SDK
      # licence.  The SDK is a fixed-output fetch, so there is no build-time
      # download and no Docker.  The same flag gates the MSVC release row in CI.
      windowsMsvcEnabled =
        (builtins.fromJSON (builtins.readFile ./rust-template.json)).windows-msvc
        or false;
      windowsMsvcCrossPackages = foundation.lib.mkWindowsMsvcCrossPackages {
        inherit self pkgs system crates crane commonArgs;
        xwinSdk =
          if windowsMsvcEnabled
          then foundation.lib.xwinSdk {inherit pkgs;}
          else null;
      };
      packages =
        rustPackages.packages
        // muslPackages
        // gnuPortablePackages
        // darwinCrossPackages
        // windowsCrossPackages
        // windowsMsvcCrossPackages
        // {
          default =
            craneLib.buildPackage (commonArgs // {pname = "rust-template";});
        };
      # The arm64 subset of the darwin cross outputs — the only ones re-signed
      # (and so the only ones the signature guard below verifies).  Empty
      # except on x86_64-linux.
      aarch64DarwinPackages =
        nixpkgs.lib.filterAttrs
        (name: _: nixpkgs.lib.hasSuffix "-aarch64-darwin" name)
        darwinCrossPackages;
      # The x86_64 subset of the Windows cross outputs, smoke-tested under wine.
      # These are non-empty on every host (the Windows helper is host-agnostic),
      # so the wine check below is gated on `system == "x86_64-linux"` rather
      # than on emptiness: wine runs a win64 PE reliably only there.
      windowsX86Packages =
        nixpkgs.lib.filterAttrs
        (name: _: nixpkgs.lib.hasSuffix "-x86_64-windows" name)
        windowsCrossPackages;
    in {
      inherit packages;
      inherit (rustPackages) apps;
      # Add the darwin ad-hoc signature guard to the workspace's checks on
      # x86_64-linux, where the zig-cross darwin binaries are produced.
      # mkDarwinCrossPackages re-signs each arm64 binary after the release
      # profile's `strip = true` would otherwise invalidate zig's link-time
      # signature; an arm64 Mach-O with an invalid signature is SIGKILLed by
      # the kernel with no output, so this check proves the shipped signature
      # is intact.  Only the arm64 outputs are checked — x86_64 macOS does not
      # enforce signatures, so those binaries ship unsigned.  Empty (and so
      # absent) on every other system.
      checks =
        rustPackages.checks
        // nixpkgs.lib.optionalAttrs (aarch64DarwinPackages != {}) {
          darwinSignatures = foundation.lib.mkDarwinSignatureCheck {
            inherit pkgs;
            darwinPackages = aarch64DarwinPackages;
          };
        }
        # Run the x86_64 Windows cross binaries under wine to prove they
        # execute, not merely link.  Gated to x86_64-linux: wine cannot exec an
        # aarch64 PE and is unreliable on Apple Silicon, so aarch64 Windows is
        # build-verified only.
        // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          windowsSmoke = foundation.lib.mkWindowsSmokeCheck {
            inherit pkgs;
            windowsPackages = windowsX86Packages;
          };
        };
      devShells = {
        default = pkgs.mkShell {
          buildInputs = [
            # Rust toolchain (compiler, cargo, rustfmt, rust-analyzer).
            rust
            # Prunes stale per-profile artifacts from target/ to reclaim disk.
            pkgs.cargo-sweep
            # JSON parsing for the shellHook's cargo-package listing and ad-hoc
            # scripting in the dev shell.
            pkgs.jq
            # Elm toolchain for the frontend/ app: compiler, formatter, and the
            # elm2nix bridge that pins Elm deps for reproducible builds.
            pkgs.elmPackages.elm
            pkgs.elmPackages.elm-format
            pkgs.elm2nix
            # Unified formatter and the per-language binaries it invokes.
            pkgs.treefmt
            pkgs.alejandra
            pkgs.prettier
            # Command runner for the project's justfile recipes.
            pkgs.just
            # Rolls the CHANGELOG on release; used by the reusable CI workflow's
            # `changelog` job and runnable locally for the same flow.
            changelog-roller.packages.${system}.default
            # Formats org-mode documents (treefmt delegates .org files to it).
            org-fmt.packages.${system}.default
            # ABI baseline check used by the reusable CI workflow's `abi`
            # job.  Compares the workspace's current public API against the
            # previous version on crates.io and reports breaking changes;
            # the job then gates on an Upcoming → Breaking changelog entry
            # when a break is detected.  Provided here so contributors can
            # run `nix develop --command cargo semver-checks ...` locally
            # before opening a PR.
            #
            # `doCheck = false` skips upstream's `target_feature_*`
            # snapshot tests, which assert against snapshots recorded on
            # x86_64 and therefore fail when building on aarch64-darwin.
            # We only ship the binary, not its test suite, so disabling
            # the check phase does not affect what the workflow runs.
            (pkgs.cargo-semver-checks.overrideAttrs (_: {doCheck = false;}))
          ];
          shellHook = ''
            ${foundation.lib.cargoHuskyHookSnippet pkgs}
            echo "Rust Template development environment"
            echo ""
            echo "Available Cargo packages (use 'cargo build -p <name>'):"
            cargo metadata --no-deps --format-version 1 2>/dev/null | \
              jq --raw-output '.packages[].name' | \
              sort | \
              sed 's/^/  • /' || echo "  Run 'cargo init' to get started"

            echo ""
            echo "Elm frontend (frontend/):"
            echo "  Build:   cd frontend && elm make src/Main.elm --output public/elm.js"
            echo "  Format:  treefmt"
            echo "  After changing elm.json dependency versions, regenerate Nix files:"
            echo "    cd frontend"
            echo "    elm2nix convert 2>/dev/null > elm-srcs.nix"
            echo "    elm2nix snapshot"
            echo "    git add elm-srcs.nix registry.dat && git commit"
          '';
          # A runtime marker identifying this as rust-template's default dev
          # shell.  A compliance check reads it back with `nix eval` to
          # confirm this shell evaluates and carries the marker; the `ci`
          # shell carries the same marker with the value "ci".
          RUST_TEMPLATE_SHELL = "default";
        };
        # Minimal shell for the reusable CI workflow: the Rust toolchain
        # plus the release CLIs the `nix develop .#ci` jobs invoke.  It
        # omits the interactive dev shell's extras (the Elm toolchain, the
        # treefmt formatter stack, just), so it is cheaper to realize; the
        # Elm frontend is a package-build input under `nix build`, not
        # something a devShell provides.  Its baseline comes from
        # foundation's mkCiShell: the same `rust` toolchain the dev shell
        # uses (so CI compiles and lints with the project's pinned
        # toolchain), changelog-roller (the `changelog` and `abi` jobs,
        # the publish flow, and dependabot-automerge all shell out to it),
        # and cargo-semver-checks (the `abi` job's crates.io ABI gate).
        # Override any of those or add release tooling via the helper's
        # arguments; see mkCiShell in foundation for the full contract.
        ci = foundation.lib.mkCiShell {
          inherit pkgs system;
          toolchain = rust;
        };
      };
    });
  in {
    devShells =
      nixpkgs.lib.mapAttrs (_: p: p.devShells) perSystem;
    packages = nixpkgs.lib.mapAttrs (_: p: p.packages) perSystem;
    apps = nixpkgs.lib.mapAttrs (_: p: p.apps) perSystem;
    checks = nixpkgs.lib.mapAttrs (_: p: p.checks) perSystem;

    # ================================================================
    # NIXOS MODULES
    # ================================================================
    nixosModules = {
      server = import ./nix/modules/nixos-server.nix {
        inherit self foundation;
      };
      default = self.nixosModules.server;
    };

    # ================================================================
    # DARWIN MODULES
    # ================================================================
    darwinModules = {
      server = import ./nix/modules/darwin-server.nix {
        inherit self foundation;
      };
      default = self.darwinModules.server;
    };
  };
}
