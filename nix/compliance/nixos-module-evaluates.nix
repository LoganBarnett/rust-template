# Evaluate a spawn's NixOS module inside a full NixOS configuration.
#
# Backs the `nixos-module-evaluates` compliance check.  The evaluation lives
# in `nix/lib/evalNixosService.nix`, which forces the module's `config` side
# and runs on any host.
#
# Invoked as:
#
#   nix-instantiate --eval --strict --json \
#     --extra-experimental-features "nix-command flakes" \
#     nix/compliance/nixos-module-evaluates.nix \
#     --argstr spawn /path/to/spawn \
#     --argstr module nixosModules.server
#
# Prints a JSON string: "ok" when the module evaluates inside the configuration,
# or "fail: …" when the flake exposes no such module.
{
  spawn,
  module,
}: let
  flake = builtins.getFlake ("path:" + spawn);
  modulePath = lib.splitString "." module;
  nixpkgs = flake.inputs.nixpkgs;
  lib = nixpkgs.lib;
in
  if !(lib.hasAttrByPath modulePath flake)
  then "fail: flake exposes no ${module}"
  else let
    serviceModule = lib.getAttrFromPath modulePath flake;
    name = import ./service-name.nix {
      inherit lib;
      pkgs = nixpkgs.legacyPackages.${builtins.currentSystem};
      module = serviceModule;
    };
  in
    builtins.seq (import ../lib/evalNixosService.nix {
      inherit nixpkgs name;
      module = serviceModule;
    }) "ok"
