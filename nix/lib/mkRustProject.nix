# mkRustProject — generate flake outputs for a Rust workspace project.
#
# Absorbs the crane build logic, devShell assembly, package generation,
# and app wiring that would otherwise be duplicated in every spawned
# project's flake.nix.
#
# Usage:
#
#   let project = inputs.foundation.lib.mkRustProject {
#     inherit self nixpkgs rust-overlay crane;
#     name = "my-app";
#     crates = {
#       server = { name = "my-app-server"; binary = "my-app-server"; };
#       cli    = { name = "my-app-cli";    binary = "my-app-cli"; };
#     };
#     extraDevPackages = system: pkgs: [
#       pkgs.cargo-sweep
#       pkgs.jq
#     ];
#     shellHook = pkgs: ''
#       echo "Welcome to my-app dev environment"
#     '';
#   };
#   in project // {
#     nixosModules = { ... };
#     darwinModules = { ... };
#   };
#
# Returns: { devShells, packages, apps }
#
# Per-crate package overrides are supported: if nix/packages/<key>.nix
# exists in the project, it is imported with { craneLib, commonArgs, pkgs }
# instead of using the generic crane build.
{
  # Required: flake self reference (used for src and package refs).
  self,
  # Required: nixpkgs input.
  nixpkgs,
  # Required: rust-overlay input.
  rust-overlay,
  # Required: crane input.
  crane,
  # Required: project name (used for the default package pname).
  name,
  # Required: workspace crate map.
  # Format: { key = { name, binary, description? }; ... }
  crates,
  # Optional: extra devShell build inputs beyond the Rust toolchain.
  # Signature: system -> pkgs -> [derivation]
  extraDevPackages ? _system: _pkgs: [],
  # Optional: shell hook appended after the standard git-hook setup.
  # Signature: pkgs -> string
  shellHook ? _pkgs: "",
  # Optional: extra buildInputs passed to all crane builds.
  extraBuildInputs ? [],
  # Optional: extra nativeBuildInputs passed to all crane builds.
  extraNativeBuildInputs ? [],
}: let
  forAllSystems =
    nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
  overlays = [(import rust-overlay)];
  pkgsFor = system:
    import nixpkgs {
      inherit system;
      overlays = overlays;
    };
in {
  devShells = forAllSystems (system: let
    pkgs = pkgsFor system;
    rust = pkgs.rust-bin.stable.latest.default.override {
      extensions = [
        "rust-src"
        "rust-analyzer"
        "rustfmt"
      ];
    };
  in {
    default = pkgs.mkShell {
      buildInputs = [rust] ++ (extraDevPackages system pkgs);
      shellHook = ''
        # Symlink cargo-husky hooks into .git/hooks/ using paths
        # relative to .git/hooks/ so the repo stays valid after moves.
        _git_root=$(git rev-parse --show-toplevel 2>/dev/null)
        if [ -n "$_git_root" ] \
            && [ "$(pwd)" = "$_git_root" ] \
            && [ -d ".cargo-husky/hooks" ]; then
          for _hook in .cargo-husky/hooks/*; do
            [ -x "$_hook" ] || continue
            _name=$(basename "$_hook")
            _dest="$_git_root/.git/hooks/$_name"
            _target=$(${pkgs.coreutils}/bin/realpath \
              --relative-to="$_git_root/.git/hooks" "$(pwd)/$_hook")
            if [ ! -L "$_dest" ] \
                || [ "$(readlink "$_dest")" != "$_target" ]; then
              ln -sf "$_target" "$_dest"
              echo "Installed git hook: $_name -> $_target"
            fi
          done
        fi

        ${shellHook pkgs}
      '';
    };
  });

  packages = forAllSystems (system: let
    pkgs = pkgsFor system;
    craneLib =
      (crane.mkLib pkgs).overrideToolchain
      (p: p.rust-bin.stable.latest.default);

    commonArgs = {
      src = craneLib.cleanCargoSource self;
      buildInputs = extraBuildInputs;
      nativeBuildInputs = extraNativeBuildInputs;
      # Run only unit tests (--lib --bins), skip integration tests
      # in tests/ directories.  Integration tests may require
      # external services not available in the Nix sandbox.
      cargoTestExtraArgs = "--lib --bins";
    };

    # Build individual crate packages from the workspace crate map.
    # When a per-crate file exists under nix/packages/, it is used
    # instead of the generic crane build; this lets individual crates
    # carry custom build options (e.g. Elm frontend bundling) without
    # cluttering the top-level flake.
    cratePackages =
      pkgs.lib.mapAttrs (
        key: crate: let
          pkgFile = self + "/nix/packages/${key}.nix";
        in
          if builtins.pathExists pkgFile
          then import pkgFile {inherit craneLib commonArgs pkgs;}
          else
            craneLib.buildPackage (commonArgs
              // {
                pname = crate.name;
                cargoExtraArgs = "-p ${crate.name}";
              })
      )
      crates;
  in
    cratePackages
    // {
      default =
        craneLib.buildPackage (commonArgs // {pname = name;});
    });

  apps = forAllSystems (system: let
    pkgs = pkgsFor system;
  in
    pkgs.lib.mapAttrs (key: crate: {
      type = "app";
      program = "${self.packages.${system}.${key}}/bin/${crate.binary}";
    })
    crates);
}
