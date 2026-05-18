# mkRustProject — generate flake outputs for a single system of a Rust
# workspace project.  The caller owns the forAllSystems wrap and supplies
# a per-system `pkgs` (with rust-overlay applied) and `craneLib`.  Keeping
# `pkgs` in the caller's scope means every parameter that references
# packages — extraDevPackages, extraBuildInputs, shellHook — is a plain
# value rather than a function-of-pkgs callback.
#
# Usage:
#
#   outputs = { self, nixpkgs, rust-overlay, crane, foundation }: let
#     forAllSystems =
#       nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
#     perSystem = forAllSystems (system: let
#       pkgs = import nixpkgs {
#         inherit system;
#         overlays = [ (import rust-overlay) ];
#       };
#       craneLib =
#         (crane.mkLib pkgs).overrideToolchain
#         (p: p.rust-bin.stable.latest.default);
#     in foundation.lib.mkRustProject {
#       inherit self pkgs craneLib;
#       name = "my-app";
#       crates = {
#         server = { name = "my-app-server"; binary = "my-app-server"; };
#         cli    = { name = "my-app-cli";    binary = "my-app-cli"; };
#       };
#       extraDevPackages = [ pkgs.cargo-sweep pkgs.jq ];
#       extraBuildInputs =
#         nixpkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.alsa-lib ]
#         ++ nixpkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
#       shellHook = ''echo "Welcome to my-app"'';
#     });
#   in {
#     devShells =
#       nixpkgs.lib.mapAttrs (_: p: { default = p.devShell; }) perSystem;
#     packages = nixpkgs.lib.mapAttrs (_: p: p.packages) perSystem;
#     apps     = nixpkgs.lib.mapAttrs (_: p: p.apps)     perSystem;
#   };
#
# Returns: { devShell, packages, apps } — single-system, not nested by
# system.  The caller assembles `devShells.<system>`, `packages.<system>`,
# `apps.<system>` from the per-system attrset.
#
# Per-crate package overrides are supported: if nix/packages/<key>.nix
# exists in the project, it is imported with { craneLib, commonArgs, pkgs }
# instead of using the generic crane build.
{
  # Required: flake self reference (used for src and package overrides).
  self,
  # Required: nixpkgs evaluated for one system, with rust-overlay applied
  # so `pkgs.rust-bin` is available.
  pkgs,
  # Required: crane lib initialized with the desired build toolchain.
  craneLib,
  # Required: project name (used for the default package pname).
  name,
  # Required: workspace crate map.
  # Format: { key = { name, binary, description? }; ... }
  crates,
  # Optional: extra devShell packages beyond the Rust toolchain.
  extraDevPackages ? [],
  # Optional: shell hook appended after the standard git-hook setup.
  shellHook ? "",
  # Optional: extra buildInputs passed to all crane builds.
  extraBuildInputs ? [],
  # Optional: extra nativeBuildInputs passed to all crane builds.
  extraNativeBuildInputs ? [],
}: let
  rust = pkgs.rust-bin.stable.latest.default.override {
    extensions = [
      "rust-src"
      "rust-analyzer"
      "rustfmt"
    ];
  };

  commonArgs = {
    src = craneLib.cleanCargoSource self;
    buildInputs = extraBuildInputs;
    nativeBuildInputs = extraNativeBuildInputs;
    # Run only unit tests (--lib --bins), skip integration tests in
    # tests/ directories.  Integration tests may require external
    # services not available in the Nix sandbox.
    cargoTestExtraArgs = "--lib --bins";
  };

  # When a per-crate file exists under nix/packages/, it is used instead
  # of the generic crane build; this lets individual crates carry custom
  # build options (e.g. Elm frontend bundling) without cluttering the
  # top-level flake.
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

  packages =
    cratePackages
    // {
      default =
        craneLib.buildPackage (commonArgs // {pname = name;});
    };
in {
  devShell = pkgs.mkShell {
    buildInputs = [rust] ++ extraDevPackages;
    shellHook = ''
      # Symlink cargo-husky hooks into .git/hooks/ using paths relative
      # to .git/hooks/ so the repo stays valid after moves.
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

      ${shellHook}
    '';
  };

  inherit packages;

  apps =
    pkgs.lib.mapAttrs (key: crate: {
      type = "app";
      program = "${packages.${key}}/bin/${crate.binary}";
    })
    crates;
}
