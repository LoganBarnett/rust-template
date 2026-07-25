# pkgsUnfreeFor — a second nixpkgs instance that accepts the unfree,
# platform-gated Apple SDK, quarantined from a project's main build `pkgs` so
# the licence acceptance needed to evaluate `apple-sdk.src` does not widen the
# free package graph the project actually builds against.
#
# A darwin cross-build that links Apple frameworks needs `apple-sdk.src`, which
# is unfree and darwin-gated; evaluating it at all requires `allowUnfree` and
# `allowUnsupportedSystem` (the latter because the consuming `pkgs` is usually
# non-darwin, e.g. a Linux CI runner).  Setting those on the main `pkgs` would
# accept the unfree licence for the whole graph the project builds.  Instead a
# consumer builds `pkgs` normally and takes only the SDK from this quarantined
# instance:
#
#   appleSdk = (foundation.lib.pkgsUnfreeFor {
#     inherit nixpkgs system overlays;
#   }).apple-sdk.src;
#
# `overlays` should be the same list the consumer passes to its own `pkgs`
# import, so the quarantined instance stays overlay-consistent with the build
# `pkgs` — the SDK source is then the one the build would see with unfree
# enabled, not a divergent one.  The acceptance stays visible in the consumer's
# flake (this call), never hidden in the foundation library, matching how
# `xwinSdk` surfaces the Microsoft SDK licence for the opt-in MSVC path.
{
  nixpkgs,
  system,
  overlays ? [],
}:
import nixpkgs {
  inherit overlays system;
  config = {
    allowUnfree = true;
    allowUnsupportedSystem = true;
  };
}
