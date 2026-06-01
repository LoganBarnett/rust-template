# Shared seam for the rust-template-server NixOS and nix-darwin
# modules.  Both platform entrypoints (`./nixos-server.nix` and
# `./darwin-server.nix`) merge this module in via their `imports`
# lists, so any declaration added here applies on both platforms.
#
# At the moment there is nothing genuinely shared between the
# platforms beyond the project name — service activation is handled
# by `foundation.lib.mkNixosService` / `mkDarwinService` in the
# respective wrappers, and those helpers internally branch on
# systemd vs launchd.  This file exists anyway so the spawned-project
# author has a single, well-known location to drop a shared option,
# a shared default, or a cross-platform health assertion: just add
# it here and the module-merge system fans it out to both
# platforms with no restructuring on either side.
{...}: {}
