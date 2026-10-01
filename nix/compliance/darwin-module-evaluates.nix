# Evaluate a spawn's darwin module inside a full nix-darwin configuration.
#
# Backs the `darwin-module-evaluates` compliance check.  The evaluation lives
# in `nix/lib/evalDarwinService.nix`, which forces the module's `config` side
# and runs on any host.
#
# Invoked as:
#
#   nix-instantiate --eval --strict --json \
#     --extra-experimental-features "nix-command flakes" \
#     nix/compliance/darwin-module-evaluates.nix \
#     --argstr spawn /path/to/spawn \
#     --argstr module darwinModules.server
#
# Prints a JSON string: "ok" when the module evaluates inside the configuration,
# or "fail: …" when the flake exposes no such module or carries no nix-darwin
# to evaluate it with.
{
  spawn,
  module,
}: let
  flake = builtins.getFlake ("path:" + spawn);
  lib = flake.inputs.nixpkgs.lib;
  modulePath = lib.splitString "." module;
  nix-darwin = flake.inputs.foundation.inputs.nix-darwin or null;
in
  if !(lib.hasAttrByPath modulePath flake)
  then "fail: flake exposes no ${module}"
  else if nix-darwin == null
  then "fail: the foundation input carries no nix-darwin; bump foundation"
  else let
    serviceModule = lib.getAttrFromPath modulePath flake;
    name = import ./service-name.nix {
      inherit lib;
      pkgs = flake.inputs.nixpkgs.legacyPackages.${builtins.currentSystem};
      module = serviceModule;
    };
  in
    builtins.seq (import ../lib/evalDarwinService.nix {
      inherit nix-darwin name;
      module = serviceModule;
    }) "ok"
