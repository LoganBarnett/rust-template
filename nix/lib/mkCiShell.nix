# mkCiShell — the baseline CI/release devShell shared by every spawned
# project and by rust-template itself.  It carries the Rust toolchain and
# the release utilities the reusable CI workflow shells out to
# (changelog-roller for CHANGELOG rolling and checking, cargo-semver-checks
# for the ABI gate), so `nix develop .#ci --command ...` has them without
# each project restating the list.
#
# The function is curried: foundation binds its own `changelog-roller`
# input once when constructing `foundation.lib.mkCiShell`, and a consumer
# then supplies per-project arguments:
#
#   ci = foundation.lib.mkCiShell {
#     inherit pkgs system;
#     toolchain = rust;                 # the project's own toolchain
#   };
#
# Overriding baseline packages: `toolchain`, `changelogRoller`, and
# `semverChecks` are ordinary formals with defaults, so a consumer swaps
# any of them by passing its own derivation.  `buildInputs` is *added* to
# the baseline rather than replacing it, and `shellHook` is *appended* to
# the baseline hook, so a consumer extends the shell without losing the
# release tooling.  Every other attribute — environment variables,
# `nativeBuildInputs`, and so on — passes straight through to `mkShell`,
# making all devShell fields available as overrides.  To diverge
# completely, a consumer stops calling this helper and writes its own
# `mkShell`.
{changelog-roller}: {
  pkgs,
  system,
  toolchain ?
    pkgs.rust-bin.stable.latest.default.override {
      extensions = [
        "rust-src"
        "rust-analyzer"
        "rustfmt"
      ];
    },
  changelogRoller ? changelog-roller.packages.${system}.default,
  semverChecks ? pkgs.cargo-semver-checks.overrideAttrs (_: {doCheck = false;}),
  buildInputs ? [],
  shellHook ? "",
  ...
} @ args: let
  # Everything the caller passed that we do not merge specially flows
  # verbatim into mkShell, so any devShell field (env vars,
  # nativeBuildInputs, ...) is overridable.
  passthrough = builtins.removeAttrs args [
    "pkgs"
    "system"
    "toolchain"
    "changelogRoller"
    "semverChecks"
    "buildInputs"
    "shellHook"
  ];
  baselinePackages = [
    # The Rust toolchain (cargo, clippy, rustfmt) every CI job builds
    # and lints with.
    toolchain
    # Rolls and checks the CHANGELOG; the `changelog` and `abi` jobs, the
    # publish flow, and dependabot-automerge all shell out to it.
    changelogRoller
    # The ABI baseline gate the `abi` job runs against crates.io.
    semverChecks
  ];
  # The baseline CI hook is intentionally empty — CI does not want the
  # git-hook install the dev shell performs.  Kept as an explicit
  # concatenation so a non-empty baseline can be introduced later without
  # touching the call sites.
  baselineHook = "";
in
  pkgs.mkShell (passthrough
    // {
      buildInputs = baselinePackages ++ buildInputs;
      shellHook = baselineHook + shellHook;
      # A runtime marker identifying this as rust-template's CI shell.  It
      # is a real environment variable (visible as $RUST_TEMPLATE_SHELL
      # inside the shell); a compliance check reads it back with `nix eval`
      # to confirm this shell evaluates and carries the marker.  Set on the
      # right of the merge so a consumer's pass-through cannot clobber it;
      # the emitted default dev shell carries the same marker with the
      # value "default".
      RUST_TEMPLATE_SHELL = "ci";
    })
