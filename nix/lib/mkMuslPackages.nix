# mkMuslPackages — statically-linked musl variants of a workspace's binaries
# for one system.
#
# A binary dynamically linked against glibc only runs where a compatible glibc
# is present, which excludes musl-based distributions and older releases.  A
# musl build links the C runtime statically, producing an artifact that runs on
# any Linux system with no runtime libc dependency.  This helper builds such a
# variant for every binary the workspace produces and names it `<name>-musl`.
#
# Only Linux systems have a musl target; for any other system the helper returns
# an empty set, so a caller can merge its result unconditionally.
#
# The musl build skips the test phase: the same sources are already exercised by
# the native build and the workspace test check in the same release, so this is
# a repackage for a different libc rather than new code to gate.
#
# Usage (mirrors mkRustPackages — the caller already has these in scope):
#
#   packages =
#     rustPackages.packages
#     // mkMuslPackages {inherit self pkgs system crates crane;}
#     // {default = ...;};
#
# Returns a single-system attrset of `<name>-musl` packages (empty on
# non-Linux).
{
  # Flake `self` — forwarded to mkRustPackages for `cleanCargoSource` and
  # per-crate override resolution.
  self,
  # Per-system nixpkgs with the rust-overlay applied (for `rust-bin`).
  pkgs,
  # The Nix system being built for; selects the musl target triple.
  system,
  # Workspace crate map, the same value passed to mkRustPackages.
  crates,
  # The crane flake input, used to build a musl-targeted crane lib.
  crane,
}: let
  mkRustPackages = import ./mkRustPackages.nix;
  # The musl target triple for each Linux system.  Systems absent here have no
  # musl variant.
  muslTargetFor = {
    x86_64-linux = "x86_64-unknown-linux-musl";
    aarch64-linux = "aarch64-unknown-linux-musl";
  };
in
  if !(muslTargetFor ? ${system})
  then {}
  else let
    muslTarget = muslTargetFor.${system};
    craneLib =
      (crane.mkLib pkgs).overrideToolchain
      (p: p.rust-bin.stable.latest.default.override {targets = [muslTarget];});
    commonArgs = {
      src = craneLib.cleanCargoSource self;
      CARGO_BUILD_TARGET = muslTarget;
      # musl targets link the C runtime statically by default; state it
      # explicitly so the intent survives a toolchain default change.
      CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
      doCheck = false;
    };
    muslPackages =
      (mkRustPackages {inherit self pkgs craneLib crates commonArgs;}).packages;
  in
    pkgs.lib.mapAttrs'
    (name: pkg: pkgs.lib.nameValuePair "${name}-musl" pkg)
    muslPackages
