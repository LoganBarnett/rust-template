# Read one module option's default out of a spawn's flake.
#
# Backs the `nix-module-option-default` compliance check.  The check asserts an
# outcome — "this option exists and defaults to this value" — rather than that
# some call was written in some file, so it evaluates the module the way a
# nix-darwin or NixOS configuration would and inspects the resulting option
# tree.  A spawn gets the right defaults by importing the foundation's service
# helper; how it got them is not this file's business.
#
# Invoked as:
#
#   nix-instantiate --eval --strict --json \
#     --extra-experimental-features "nix-command flakes" \
#     nix/compliance/module-option-default.nix \
#     --argstr spawn /path/to/spawn \
#     --argstr module darwinModules.server \
#     --argstr option logPathStdout \
#     --argstr expected '/var/log/@service@-stdout.log'
#
# nix-instantiate rather than `nix eval` because only the former applies
# --argstr to a file that evaluates to a function; `nix eval --file` hands back
# the uncalled lambda.  The alternative — splicing the arguments into a
# `nix eval --expr` string — is what --argstr exists to avoid, so no
# caller-supplied text is ever spliced into an expression here.
#
# Prints a JSON string: "ok" when the option holds the expected default,
# "skip: …" when the check does not apply to this spawn, or "fail: …"
# describing the drift.
#
# `@service@` in `expected` is replaced with the service name discovered from
# the module.  The name varies per spawn (it is `<project>-server`), and the
# checker has no way to learn it — SpawnContext carries the spawn's directory,
# not its project name — so the module is asked rather than told.
{
  spawn,
  module,
  option,
  expected,
}: let
  flake = builtins.getFlake ("path:" + spawn);
  # The spawn's own nixpkgs, so the module is evaluated against the lib it was
  # written for rather than whatever the checker happens to have pinned.
  lib = flake.inputs.nixpkgs.lib;
  modulePath = lib.splitString "." module;
  optionPath = lib.splitString "." option;
in
  if !(lib.hasAttrByPath modulePath flake)
  then "skip: flake exposes no ${module}"
  else let
    # `_module.check = false` keeps the module's config-side declarations
    # (launchd.*, users.*, environment.etc) from needing a host module set to
    # merge against.  Only `options` is ever forced below; `config` is never
    # touched, so nothing in the service's config block is evaluated.
    evaluated = lib.evalModules {
      modules = [
        (lib.getAttrFromPath modulePath flake)
        {_module.check = false;}
      ];
      specialArgs = {
        pkgs = flake.inputs.nixpkgs.legacyPackages.${builtins.currentSystem};
      };
    };
    services = evaluated.options.services or {};
    names = builtins.attrNames services;
  in
    if names == []
    then "fail: ${module} declares no services.<name> options"
    else let
      service = builtins.head names;
      declared = lib.attrByPath ([service] ++ optionPath) null services;
      want = builtins.replaceStrings ["@service@"] [service] expected;
    in
      if declared == null
      then "fail: services.${service} declares no option ${option}"
      else if !(declared ? default)
      then "fail: services.${service}.${option} declares no default"
      else if declared.default == want
      then "ok"
      else
        "fail: services.${service}.${option} defaults to "
        + "\"${toString declared.default}\", expected \"${want}\""
