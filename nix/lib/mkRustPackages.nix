# mkRustPackages — iterate over a workspace crate map and emit
# `{ packages, apps, checks }` for a single system.
#
# This helper deliberately does only the iteration: it does not build
# `commonArgs`, set up a devShell, or construct a Rust toolchain.  The
# caller assembles `commonArgs` in lexical scope so every crane attribute
# (env vars, build inputs, doCheck, ...) is a plain field on a value the
# caller owns — no `extraX` parameter is needed to thread a new crane knob
# through the helper.
#
# Test scope is chosen per crate rather than taken from `commonArgs`: a
# bin-only crate has no library target, so `cargo test --lib` would error,
# and a crate with a `src/lib.rs` runs both its lib and bin unit-test sets.
# Integration tests under `tests/` are always skipped here because they may
# need services unavailable in the Nix sandbox; the `checks.workspace-tests`
# derivation covers every member's unit tests, including the library crates
# that are dependencies rather than packages and so are never exercised by a
# per-binary build.
#
# Per-crate override convention: if `nix/packages/<key>.nix` exists in the
# project, it is imported with `{ craneLib, commonArgs, pkgs }` instead of
# using the generic crane build.  This lets one crate (e.g. a server with
# a bundled Elm frontend) carry custom build logic without disturbing the
# others.  The `commonArgs` handed to the override is enriched with that
# crate's shared dependency artifacts and test scope, so the override needs
# no awareness of either.
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
#       env = { my_app_health_tests_strict = "false"; };
#     };
#     inherit (foundation.lib.mkRustPackages {
#       inherit self pkgs craneLib crates commonArgs;
#     }) packages apps checks;
#   in { inherit packages apps checks; devShell = ...; });
#
# Returns `{ packages, apps, checks }` — single-system, not nested by system.
{
  # Flake `self` — used to resolve per-crate package override files at
  # `self + "/nix/packages/<key>.nix"`, to detect a crate's library target
  # at `self + "/crates/<key>/src/lib.rs"`, and for `cleanCargoSource`.
  self,
  # Per-system nixpkgs (only forwarded to per-crate override files; the
  # generic crane build does not use it directly).
  pkgs,
  # Crane lib initialized with the desired toolchain.
  craneLib,
  # Workspace crate map: `{ key = { name; binary; description?; }; ... }`.
  crates,
  # Crane common arguments — `src`, `buildInputs`, `nativeBuildInputs`,
  # `env`, etc.  Assembled by the caller.  Test scope is set per crate here,
  # so a `cargoTestExtraArgs` in `commonArgs` is overridden.
  commonArgs,
}: let
  # Build the workspace's dependencies once and share the result across every
  # package and the workspace test check, rather than recompiling them per
  # crate.
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  # Per-crate crane arguments: the shared dependency artifacts plus a test
  # scope chosen from whether the crate has a library target.
  crateArgs = key:
    commonArgs
    // {
      inherit cargoArtifacts;
      cargoTestExtraArgs =
        if builtins.pathExists (self + "/crates/${key}/src/lib.rs")
        then "--lib --bins"
        else "--bins";
    };

  packages =
    pkgs.lib.mapAttrs (
      key: crate: let
        pkgFile = self + "/nix/packages/${key}.nix";
        args = crateArgs key;
      in
        if builtins.pathExists pkgFile
        then
          import pkgFile {
            inherit craneLib pkgs;
            commonArgs = args;
          }
        else
          craneLib.buildPackage (args
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

  # Workspace-wide unit tests, run by `nix flake check` (building a check
  # derivation runs its phases, so the tests execute).  This is the only place
  # the library crates' unit tests run under Nix, since they are dependencies
  # rather than packages.  Binary crates' unit tests run here too as well as in
  # their own package build — the redundancy is accepted so a plain `nix build`
  # of a binary still gates on its tests.
  checks = {
    workspace-tests = craneLib.cargoTest (commonArgs
      // {
        inherit cargoArtifacts;
        pname = "workspace-tests";
        cargoTestExtraArgs = "--workspace --lib --bins";
      });
  };
in {
  inherit packages apps checks;
}
