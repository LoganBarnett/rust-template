# Discover the service name a module declares under `services.<name>`.
#
# The name varies per spawn (it is `<project>-server`) and the checker has no
# way to learn it.
#
# Returns the name, or throws when the module declares no service.
{
  # The spawn's nixpkgs `lib`.
  lib,
  # The spawn's nixpkgs package set for the checker's own system.
  pkgs,
  # The service module, e.g. a spawn's `darwinModules.server`.
  module,
}: let
  evaluated = lib.evalModules {
    modules = [module {_module.check = false;}];
    specialArgs = {inherit pkgs;};
  };
  names = builtins.attrNames (evaluated.options.services or {});
in
  if names == []
  then throw "the module declares no services.<name> options"
  else builtins.head names
