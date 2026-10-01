# mkDarwinServiceEvalCheck — a `nix flake check` derivation proving a service
# module evaluates under nix-darwin with the daemons it generates forced.
{
  pkgs,
  nix-darwin,
  # The service module to evaluate, e.g. a flake's `darwinModules.server`.
  module,
  # The service name the module declares under `services.<name>`.
  name,
}:
pkgs.writeText "darwin-service-eval-${name}.json"
# Keeping the string context on the stub package's store path would make this
# derivation depend on building an aarch64-darwin derivation, which a Linux
# builder cannot do.  The check's subject is evaluation, not a closure, so the
# context is dropped and the text only records what was forced.  The
# evaluation itself lives in evalDarwinService.nix, shared with the
# darwin-module-evaluates compliance helper; this only turns its result into
# something `nix build` can realize.
(builtins.unsafeDiscardStringContext
  (import ./evalDarwinService.nix {inherit nix-darwin module name;}))
