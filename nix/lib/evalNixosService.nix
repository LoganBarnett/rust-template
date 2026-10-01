# evalNixosService — evaluate a service module inside a full NixOS
# configuration and return the systemd units it generates, forced, as JSON.
#
# Pure and host-agnostic: an x86_64-linux configuration evaluates on a Mac as
# readily as on a Linux builder, so neither caller gates on the host system.
{
  # The nixpkgs flake whose `lib.nixosSystem` evaluates the configuration.
  nixpkgs,
  # The service module to evaluate, e.g. a spawn's `nixosModules.server`.
  module,
  # The service name the module declares under `services.<name>`.
  name,
}: let
  lib = nixpkgs.lib;
  probe = {pkgs, ...}: {
    nixpkgs.hostPlatform = "x86_64-linux";
    system.stateVersion = "25.11";
    services.${name} = {
      enable = true;
      # A stub package keeps the evaluation about option wiring; the default
      # is the spawn's crane build, which the flake-output checks already
      # cover.
      package = pkgs.writeShellScriptBin name ":";
    };
  };
  systemd =
    (nixpkgs.lib.nixosSystem {modules = [module probe];})
    .config
    .systemd;
in
  builtins.toJSON ({service = systemd.services.${name}.serviceConfig;}
    // lib.optionalAttrs (systemd.sockets ? ${name}) {
      socket = systemd.sockets.${name}.socketConfig;
    })
