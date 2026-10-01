# mkNixosServiceEvalCheck — a `nix flake check` derivation proving a service
# module evaluates under NixOS with the systemd units it generates forced.
{
  pkgs,
  nixpkgs,
  # The service module to evaluate, e.g. a flake's `nixosModules.server`.
  module,
  # The service name the module declares under `services.<name>`.
  name,
}:
pkgs.writeText "nixos-service-eval-${name}.json"
# Keeping the string context on the stub package's store path would make this
# derivation depend on building an x86_64-linux derivation, which a Mac running
# `nix flake check` cannot do.  The check's subject is evaluation, not a
# closure, so the context is dropped and the text only records what was forced.
# The evaluation itself lives in evalNixosService.nix, shared with the
# nixos-module-evaluates compliance helper; this only turns its result into
# something `nix build` can realize.
(builtins.unsafeDiscardStringContext
  (import ./evalNixosService.nix {inherit nixpkgs module name;}))
