# mkRustPackages — iterate over a workspace crate map and emit
# `{ packages, apps }` for a single system.
#
# This helper deliberately does only the iteration: it does not build
# `commonArgs`, set up a devShell, or construct a Rust toolchain.  The
# caller assembles `commonArgs` in lexical scope so every crane attribute
# (env vars, build inputs, cargoTestExtraArgs, doCheck, ...) is a plain
# field on a value the caller owns — no `extraX` parameter is needed to
# thread a new crane knob through the helper.
#
# Per-crate override convention: if `nix/packages/<key>.nix` exists in the
# project, it is imported with `{ craneLib, commonArgs, pkgs }` instead of
# using the generic crane build.  This lets one crate (e.g. a server with
# a bundled Elm frontend) carry custom build logic without disturbing the
# others.
#
# Usage:
#
#   perSystem = forAllSystems (system: let
#     pkgs = import nixpkgs { inherit system; overlays = [...]; };
#     craneLib = (crane.mkLib pkgs).overrideToolchain (...);
#     crates = {
#       server = { name = "my-app-server"; binary = "my-app-server"; };
#       cli    = { name = "my-app-cli";    binary = "my-app-cli"; };
#     };
#     commonArgs = {
#       src = craneLib.cleanCargoSource self;
#       cargoTestExtraArgs = "--lib --bins";
#       env = { my_app_health_tests_strict = "false"; };
#     };
#     inherit (foundation.lib.mkRustPackages {
#       inherit self pkgs craneLib crates commonArgs;
#     }) packages apps;
#   in { inherit packages apps; devShell = ...; });
#
# Returns `{ packages, apps }` — single-system, not nested by system.
{
  # Flake `self` — used to resolve per-crate package override files at
  # `self + "/nix/packages/<key>.nix"` and for `cleanCargoSource`.
  self,
  # Per-system nixpkgs (only forwarded to per-crate override files; the
  # generic crane build does not use it directly).
  pkgs,
  # Crane lib initialized with the desired toolchain.
  craneLib,
  # Workspace crate map: `{ key = { name; binary; description?; }; ... }`.
  crates,
  # Crane common arguments — `src`, `buildInputs`, `nativeBuildInputs`,
  # `env`, `cargoTestExtraArgs`, etc.  Assembled by the caller.
  commonArgs,
}: let
  packages =
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

  apps =
    pkgs.lib.mapAttrs (key: crate: {
      type = "app";
      program = "${packages.${key}}/bin/${crate.binary}";
    })
    crates;
in {
  inherit packages apps;
}
