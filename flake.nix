# Unfortunately we need to duplicate much of what is in template/flake.nix
# because the pre-commit hooks that get installed need things like rustfmt on
# the path.
{
  description = "A Rust repository template.";
  inputs = {
    # LLM: Do NOT change this URL unless explicitly directed. This is the
    # correct format for nixpkgs stable (25.11 is correct, not nixos-25.11).
    nixpkgs.url = "github:NixOS/nixpkgs/25.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    changelog-roller.url = "github:LoganBarnett/changelog-roller";
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
    org-fmt,
  } @ inputs: let
    forAllSystems = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
    overlays = [
      (import rust-overlay)
    ];
    pkgsFor = system: import nixpkgs {inherit overlays system;};
    # Same nixpkgs, but accepting the unfree, darwin-gated Apple SDK so the
    # cross-fixture's framework-linking build can evaluate `apple-sdk.src` on a
    # Linux builder.  The acceptance stays visible here rather than depending on
    # a NIXPKGS_ALLOW_UNFREE env var, matching what CONTRIBUTING.org tells
    # consumers whose crates link Apple frameworks.  This delegates to the
    # shared `foundation.lib.pkgsUnfreeFor` helper so the repo dogfoods the
    # same quarantined-unfree-nixpkgs machinery spawns consume.
    pkgsUnfreeFor = system:
      import ./nix/lib/pkgsUnfreeFor.nix {inherit nixpkgs overlays system;};
    mkRustPackages = import ./nix/lib/mkRustPackages.nix;
    # The baseline CI/release shell, with foundation's changelog-roller
    # input pre-bound.  Consumed both by this repo's own `.#ci` devShell
    # below and, via `foundation.lib.mkCiShell`, by every spawned project.
    mkCiShell = import ./nix/lib/mkCiShell.nix {inherit changelog-roller org-fmt;};
    # Binary crates this repo ships as release artifacts.  Mirrors the
    # release-binary = true entries in rust-template.json: compliance-cli and
    # dependency-bump-cli are the binaries; the foundation crates and the
    # *-lib crates are libraries.
    crates = {
      compliance-cli = {
        name = "rust-template-compliance-cli";
        binary = "rust-template-compliance-cli";
      };
      dependency-bump-cli = {
        name = "rust-template-dependency-bump-cli";
        binary = "rust-template-dependency-bump-cli";
      };
    };
    # A build-only regression fixture guarding the zig-linked build paths the
    # release crates never hit: cross-compiled with the Apple SDK for the two
    # darwin paths (a C-compiling dependency compiled via zig in crane's
    # deps-only phase, and Apple framework linking), and gnu-portable-built to
    # link a modern-glibc host shared library (libasound), the case that forced
    # mkGnuPortablePackages' --allow-shlib-undefined flag.  Kept out of `crates`
    # above so it is never built native/musl or shipped as a release artifact —
    # only the darwin-cross and gnu-portable fixture attrs below reference it.
    fixtureCrates = {
      cross-fixture = {
        name = "rust-template-cross-fixture";
        binary = "rust-template-cross-fixture";
      };
    };
    # Crane-built workspace binaries for one system, assembled the same way
    # template/flake.nix builds a spawned project's cli/server so the repo
    # dogfoods its own release-binary machinery.
    rustPackagesFor = system: let
      pkgs = pkgsFor system;
      craneLib =
        (crane.mkLib pkgs).overrideToolchain
        (p: p.rust-bin.stable.latest.default);
      commonArgs = {
        src = craneLib.cleanCargoSource self;
        # mkRustPackages chooses the test scope per crate (a bin-only crate
        # would error on `cargo test --lib`) and adds a workspace-wide test
        # check, so no cargoTestExtraArgs is set here.
      };
    in
      mkRustPackages {inherit self pkgs craneLib crates commonArgs;};
    mkMuslPackages = import ./nix/lib/mkMuslPackages.nix;
    mkGnuPortablePackages = import ./nix/lib/mkGnuPortablePackages.nix;
    mkDarwinCrossPackages = import ./nix/lib/mkDarwinCrossPackages.nix;
    mkDarwinSignatureCheck = import ./nix/lib/mkDarwinSignatureCheck.nix;
    mkWindowsCrossPackages = import ./nix/lib/mkWindowsCrossPackages.nix;
    mkWindowsSmokeCheck = import ./nix/lib/mkWindowsSmokeCheck.nix;
    mkWindowsMsvcCrossPackages = import ./nix/lib/mkWindowsMsvcCrossPackages.nix;
    xwinSdk = import ./nix/lib/xwin-sdk.nix;
    devPackages = system: let
      pkgs = pkgsFor system;
      rust = pkgs.rust-bin.stable.latest.default.override {
        extensions = [
          # For rust-analyzer and others.  See
          # https://nixos.wiki/wiki/Rust#Shell.nix_example for some details.
          "rust-src"
          "rust-analyzer"
          "rustfmt"
        ];
      };
    in [
      # Rust toolchain (compiler, cargo, rustfmt, rust-analyzer); the
      # pre-commit hook needs rustfmt on PATH, hence the duplication noted
      # at the top of this file.
      rust
      # The command runner for this repo's justfile recipes, and the binary the
      # compliance checker's justfile-recipe check shells out to
      # (`just --summary`) — declared here rather than borrowed from the system.
      pkgs.just
      # Unified formatter and the per-language binaries it invokes.
      # `new-project.sh` runs `treefmt` as its final spawn step, so this
      # devShell needs to provide them when users invoke the script from
      # here.  Mirrors the set in template/flake.nix's extraDevPackages.
      pkgs.treefmt
      pkgs.alejandra
      pkgs.prettier
      pkgs.elmPackages.elm-format
      org-fmt.packages.${system}.default
      # Used by the reusable CI workflow's `changelog` job; pulled in here
      # so `nix develop --command changelog-roller ...` works when the
      # workflow runs against this repository as well as spawned projects.
      changelog-roller.packages.${system}.default
      # ABI baseline check used by the reusable CI workflow's `abi` job.
      # Compares the workspace's current public API against the previous
      # version published to crates.io and reports breaking changes; the
      # job then gates on an Upcoming → Breaking changelog entry when a
      # break is detected.  Provided here so the same `nix develop
      # --command cargo semver-checks ...` invocation works locally for
      # contributors auditing a change before opening the PR.
      #
      # `doCheck = false` skips upstream's `target_feature_*` snapshot
      # tests, which assert against snapshots recorded on x86_64 and
      # therefore fail when building on aarch64-darwin.  We only ship
      # the binary, not its test suite, so disabling the check phase
      # does not affect what the workflow runs.
      (pkgs.cargo-semver-checks.overrideAttrs (_: {doCheck = false;}))
      # The one-shot Dependabot-backlog combiner, taken from this flake's own
      # package output — the very package spawns pull in via
      # foundation.packages.<system>.dependabot-combine.
      self.packages.${system}.dependabot-combine
      # The daily dependency bumper, likewise taken from this flake's own
      # package output so `just dependency-bump` works here the same way it
      # does in a spawn.
      self.packages.${system}.dependency-bump
    ];
  in {
    devShells = forAllSystems (system: {
      default = (pkgsFor system).mkShell {
        buildInputs = devPackages system;
        # A runtime marker identifying this as rust-template's default dev
        # shell, matching what the emitted template ships; a compliance
        # check reads it back with `nix eval` to confirm the shell
        # evaluates and carries the marker.  The `ci` shell carries "ci".
        RUST_TEMPLATE_SHELL = "default";
      };
      # This repo dogfoods the CI shell it ships to spawns: the reusable
      # CI workflow runs against rust-template itself via `nix develop
      # .#ci`, so the shell must exist here too.  The baseline toolchain
      # default matches the dev shell's `rust`, so nothing is passed.
      ci = mkCiShell {
        pkgs = pkgsFor system;
        inherit system;
      };
    });

    # The repo builds its own binaries (currently just compliance-cli) so it
    # can release them through the same machinery spawned projects use.  On
    # Linux each binary also gets a statically-linked `<name>-musl` variant, and
    # the x86_64-linux build cross-compiles macOS `<key>-<arch>-darwin` variants
    # via zig.  compliance-cli is libSystem-only, so no `appleSdk` is passed and
    # its cross build stays licence-free.  The cross-fixture is built separately
    # with the Apple SDK to guard framework linking — see fixtureCrates.
    packages = forAllSystems (
      system: let
        cratePackages = (rustPackagesFor system).packages;
        muslPackages = mkMuslPackages {
          inherit self crane crates system;
          pkgs = pkgsFor system;
        };
        # Portable glibc-dynamic variant: runs off the Nix store (FHS
        # interpreter, glibc 2.17 floor) and links host shared libraries.
        # Empty except on Linux, like the musl and cross outputs.
        gnuPortablePackages = mkGnuPortablePackages {
          inherit self crane crates system;
          pkgs = pkgsFor system;
        };
        darwinCrossPackages = mkDarwinCrossPackages {
          inherit self crane crates system;
          pkgs = pkgsFor system;
        };
        # Native Windows PE variants (`<crate>-{x86_64,aarch64}-windows`),
        # cross-compiled via llvm-mingw for the gnullvm targets.  Host-agnostic
        # (see mkWindowsCrossPackages), so this builds on the Linux CI runners
        # and on a contributor's Mac alike — no `system` gate like the darwin
        # cross outputs.  compliance-cli is pure Rust, so its Windows build
        # needs nothing beyond the toolchain.
        windowsCrossPackages = mkWindowsCrossPackages {
          inherit self crane crates system;
          pkgs = pkgsFor system;
        };
        # The opt-in MSVC-ABI Windows variant
        # (`<crate>-x86_64-windows-msvc`).  This repo dogfoods the opt-in by
        # passing `xwinSdk`, so its CI exercises the MSVC path exactly as
        # fixtureDarwinPackages passes `appleSdk` — evaluating the xwin SDK here
        # accepts Microsoft's SDK licence for this repo, the visible consent the
        # opt-in is designed around.  A spawn that does not pass `xwinSdk`
        # builds no MSVC variant and accepts no licence.
        windowsMsvcCrossPackages = mkWindowsMsvcCrossPackages {
          inherit self crane crates system;
          pkgs = pkgsFor system;
          xwinSdk = xwinSdk {pkgs = pkgsFor system;};
        };
        # The fixture links Apple frameworks, so it needs the Apple SDK and the
        # unfree-accepting pkgs.  Empty except on x86_64-linux, like the other
        # cross outputs.
        fixtureDarwinPackages = mkDarwinCrossPackages {
          inherit self crane system;
          pkgs = pkgsUnfreeFor system;
          crates = fixtureCrates;
          appleSdk = (pkgsUnfreeFor system).apple-sdk.src;
        };
        # The gnu-portable analogue: builds the fixture's `<name>-gnu` variant
        # linking libasound, so a regression in the helper's
        # --allow-shlib-undefined handling — the only path that links a
        # modern-glibc host shared library — fails the build.  libasound is in
        # buildInputs so it is realized in the sandbox, and its lib directory is
        # handed to cross-fixture's build.rs via CROSS_FIXTURE_ASOUND_LIBDIR;
        # the build script emits the `-lasound` link (and the `link_asound`
        # cfg) only when that var is set, so no other build pulls the library.
        # Empty except on Linux, like the other portable outputs.
        gnuPortableFixturePackages = mkGnuPortablePackages {
          inherit self crane system;
          pkgs = pkgsFor system;
          crates = fixtureCrates;
          commonArgs = {
            buildInputs = [(pkgsFor system).alsa-lib];
            CROSS_FIXTURE_ASOUND_LIBDIR = "${(pkgsFor system).alsa-lib}/lib";
          };
        };
        # The Windows analogue: cross-builds the fixture's
        # `<name>-{x86_64,aarch64}-windows` variants via llvm-mingw, so a
        # regression in mkWindowsCrossPackages' C toolchain (the windows `ring`
        # dependency) or import-library wiring (the winmm link) fails the build.
        # Host-agnostic like the helper, so unlike the darwin fixture this also
        # builds on a contributor's Mac.
        windowsCrossFixturePackages = mkWindowsCrossPackages {
          inherit self crane system;
          pkgs = pkgsFor system;
          crates = fixtureCrates;
        };
        # The MSVC analogue: cross-builds the fixture's
        # `<name>-x86_64-windows-msvc` variant via clang-cl/lld-link against the
        # xwin SDK, so a regression in the MSVC C toolchain (the windows `ring`
        # dependency) or the SDK library wiring fails the build.
        windowsMsvcCrossFixturePackages = mkWindowsMsvcCrossPackages {
          inherit self crane system;
          pkgs = pkgsFor system;
          crates = fixtureCrates;
          xwinSdk = xwinSdk {pkgs = pkgsFor system;};
        };
      in
        cratePackages
        // muslPackages
        // gnuPortablePackages
        // darwinCrossPackages
        // windowsCrossPackages
        // windowsMsvcCrossPackages
        // fixtureDarwinPackages
        // gnuPortableFixturePackages
        // windowsCrossFixturePackages
        // windowsMsvcCrossFixturePackages
        // {
          default = cratePackages.compliance-cli;
          # One-shot Dependabot-backlog combiner, exposed so spawns include it
          # in their dev shell (foundation.packages.<system>.dependabot-combine)
          # rather than carry a copy that drifts.  Only cargo is needed from the
          # toolchain (for `cargo update --precise`), so the base toolchain is
          # passed rather than the dev shell's extended one.
          dependabot-combine = (pkgsFor system).callPackage ./nix/dependabot-combine.nix {
            rustToolchain = (pkgsFor system).rust-bin.stable.latest.default;
            changelog-roller = changelog-roller.packages.${system}.default;
          };
          # The daily dependency bumper (crates/dependency-bump-*), wrapped
          # with its runtime tools on PATH and exposed so spawns pull it from
          # foundation.packages.<system>.dependency-bump rather than carry a
          # copy that drifts.  The scheduled workflow `nix run`s this same
          # attribute at main, keeping tool and workflow in lockstep.
          dependency-bump = (pkgsFor system).callPackage ./nix/dependency-bump.nix {
            dependency-bump-cli = cratePackages.dependency-bump-cli;
            rustToolchain = (pkgsFor system).rust-bin.stable.latest.default;
            changelog-roller = changelog-roller.packages.${system}.default;
            org-fmt = org-fmt.packages.${system}.default;
          };
        }
    );

    apps = forAllSystems (system: (rustPackagesFor system).apps);

    # `nix flake check` builds these, which runs the workspace's unit tests
    # (every member's lib and bin tests, integration tests excluded) and, on
    # x86_64-linux, verifies the darwin cross binaries are validly signed and
    # runs the x86_64 Windows cross binaries under wine.
    checks = forAllSystems (
      system: let
        pkgs = pkgsFor system;
        lib = nixpkgs.lib;
        # The arm64 darwin cross outputs, keyed `<crate>-aarch64-darwin`; only
        # arm64 is signed (x86_64 macOS does not enforce signatures), and this
        # set is empty on every system but x86_64-linux, so the check below is
        # absent there.
        darwinPackages =
          lib.filterAttrs
          (name: _: lib.hasSuffix "-aarch64-darwin" name)
          self.packages.${system};
        # The x86_64 Windows cross outputs, keyed `<crate>-x86_64-windows`.
        # Unlike the darwin outputs these are non-empty on every host (the
        # Windows helper is host-agnostic), so the wine smoke-test is gated on
        # the system directly: wine runs a win64 PE reliably only on
        # x86_64-linux (it cannot exec an aarch64 PE, and is flaky on Apple
        # Silicon), so aarch64 Windows stays build-verified only.
        windowsX86Packages =
          lib.filterAttrs
          (name: _: lib.hasSuffix "-x86_64-windows" name)
          self.packages.${system};
      in
        (rustPackagesFor system).checks
        // lib.optionalAttrs (darwinPackages != {}) {
          darwinSignatures = mkDarwinSignatureCheck {inherit pkgs darwinPackages;};
        }
        // lib.optionalAttrs (system == "x86_64-linux") {
          windowsSmoke = mkWindowsSmokeCheck {
            inherit pkgs;
            windowsPackages = windowsX86Packages;
          };
        }
    );

    # Reusable helpers for spawned projects.  Imported via:
    #   inputs.foundation.lib.mkNixosService { name = "my-app-server"; self = self; }
    lib = {
      mkNixosService = import ./nix/lib/mkNixosService.nix;
      mkDarwinService = import ./nix/lib/mkDarwinService.nix;
      mkRustPackages = import ./nix/lib/mkRustPackages.nix;
      mkMuslPackages = import ./nix/lib/mkMuslPackages.nix;
      mkGnuPortablePackages = import ./nix/lib/mkGnuPortablePackages.nix;
      mkDarwinCrossPackages = import ./nix/lib/mkDarwinCrossPackages.nix;
      pkgsUnfreeFor = import ./nix/lib/pkgsUnfreeFor.nix;
      mkDarwinSignatureCheck = import ./nix/lib/mkDarwinSignatureCheck.nix;
      mkWindowsCrossPackages = import ./nix/lib/mkWindowsCrossPackages.nix;
      mkWindowsSmokeCheck = import ./nix/lib/mkWindowsSmokeCheck.nix;
      mkWindowsMsvcCrossPackages = import ./nix/lib/mkWindowsMsvcCrossPackages.nix;
      xwinSdk = import ./nix/lib/xwin-sdk.nix;
      cargoHuskyHookSnippet = import ./nix/lib/cargoHuskyHookSnippet.nix;
      inherit mkCiShell;
    };
  };
}
