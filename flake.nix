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
    # consumers whose crates link Apple frameworks.
    pkgsUnfreeFor = system:
      import nixpkgs {
        inherit overlays system;
        config = {
          allowUnfree = true;
          allowUnsupportedSystem = true;
        };
      };
    mkRustPackages = import ./nix/lib/mkRustPackages.nix;
    # The baseline CI/release shell, with foundation's changelog-roller
    # input pre-bound.  Consumed both by this repo's own `.#ci` devShell
    # below and, via `foundation.lib.mkCiShell`, by every spawned project.
    mkCiShell = import ./nix/lib/mkCiShell.nix {inherit changelog-roller;};
    # Binary crates this repo ships as release artifacts.  Mirrors the
    # release-binary = true entries in rust-template.json: compliance-cli is the
    # only binary; the foundation crates and compliance-lib are libraries.
    crates = {
      compliance-cli = {
        name = "rust-template-compliance-cli";
        binary = "rust-template-compliance-cli";
      };
    };
    # A build-only regression fixture, cross-compiled with the Apple SDK to
    # guard the two darwin paths the release crates never hit: a C-compiling
    # dependency (compiled via zig in crane's deps-only phase) and Apple
    # framework linking.  Kept out of `crates` above so it is never built
    # native/musl or shipped as a release artifact — only the darwin-cross
    # fixture attrs below reference it.
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
        # The fixture links Apple frameworks, so it needs the Apple SDK and the
        # unfree-accepting pkgs.  Empty except on x86_64-linux, like the other
        # cross outputs.
        fixtureDarwinPackages = mkDarwinCrossPackages {
          inherit self crane system;
          pkgs = pkgsUnfreeFor system;
          crates = fixtureCrates;
          appleSdk = (pkgsUnfreeFor system).apple-sdk.src;
        };
      in
        cratePackages
        // muslPackages
        // gnuPortablePackages
        // darwinCrossPackages
        // fixtureDarwinPackages
        // {default = cratePackages.compliance-cli;}
    );

    apps = forAllSystems (system: (rustPackagesFor system).apps);

    # `nix flake check` builds these, which runs the workspace's unit tests
    # (every member's lib and bin tests, integration tests excluded).
    checks = forAllSystems (system: (rustPackagesFor system).checks);

    # Reusable helpers for spawned projects.  Imported via:
    #   inputs.foundation.lib.mkNixosService { name = "my-app-server"; self = self; }
    lib = {
      mkNixosService = import ./nix/lib/mkNixosService.nix;
      mkDarwinService = import ./nix/lib/mkDarwinService.nix;
      mkRustPackages = import ./nix/lib/mkRustPackages.nix;
      mkMuslPackages = import ./nix/lib/mkMuslPackages.nix;
      mkGnuPortablePackages = import ./nix/lib/mkGnuPortablePackages.nix;
      mkDarwinCrossPackages = import ./nix/lib/mkDarwinCrossPackages.nix;
      cargoHuskyHookSnippet = import ./nix/lib/cargoHuskyHookSnippet.nix;
      inherit mkCiShell;
    };
  };
}
