# evalDarwinService — evaluate a service module inside a full nix-darwin
# configuration and return the launchd daemons it generates, forced, as JSON.
#
# Pure and host-agnostic: an aarch64-darwin configuration evaluates on a Linux
# builder as readily as on a Mac, so neither caller gates on the host system.
{
  # The nix-darwin flake — an input of the foundation flake.
  nix-darwin,
  # The service module to evaluate, e.g. a spawn's `darwinModules.server`.
  module,
  # The service name the module declares under `services.<name>`.
  name,
}: let
  lib = nix-darwin.inputs.nixpkgs.lib;
  probe = {pkgs, ...}: {
    nixpkgs.hostPlatform = "aarch64-darwin";
    system.stateVersion = 6;
    services.${name} = {
      enable = true;
      # A stub package keeps the evaluation about option wiring; the default
      # is the spawn's crane build for aarch64-darwin, which the flake-output
      # checks already cover.  The option's type is `package`, so the stub is
      # a derivation rather than a path string.
      package = pkgs.writeShellScriptBin name ":";
    };
  };
  daemons =
    (nix-darwin.lib.darwinSystem {modules = [module probe];})
    .config
    .launchd
    .daemons;
  # Every service ships a health check, so a module that defines none, or
  # disables it, fails here rather than passing on its server daemon alone.
  healthcheck =
    daemons."${name}-healthcheck"
    or (throw "services.${name} defines no ${name}-healthcheck daemon");
in
  builtins.toJSON {
    server = daemons.${name}.serviceConfig;
    healthcheck = healthcheck.serviceConfig;
  }
