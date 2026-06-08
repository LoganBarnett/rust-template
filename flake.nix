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
    packages = system: let
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
        buildInputs = packages system;
      };
    });

    # Reusable helpers for spawned projects.  Imported via:
    #   inputs.foundation.lib.mkNixosService { name = "my-app-server"; self = self; }
    lib = {
      mkNixosService = import ./nix/lib/mkNixosService.nix;
      mkDarwinService = import ./nix/lib/mkDarwinService.nix;
      mkRustPackages = import ./nix/lib/mkRustPackages.nix;
      cargoHuskyHookSnippet = import ./nix/lib/cargoHuskyHookSnippet.nix;
    };
  };
}
