# Per-crate build override for compliance-cli (see the override convention in
# nix/lib/mkRustPackages.nix).  The generic crane build compiles the CLI fine,
# but the checker's justfile-recipe check shells out to `just --summary` at
# runtime.  Wrapping the binary so `just` is on its PATH ships that dependency
# with the CLI itself, so the packaged tool works in any Nix context — not only
# a dev shell that happens to provide `just`.
{
  craneLib,
  commonArgs,
  pkgs,
}: let
  # The plain crane build, identical to the generic path in mkRustPackages.
  unwrapped = craneLib.buildPackage (commonArgs
    // {
      pname = "rust-template-compliance-cli";
      cargoExtraArgs = "-p rust-template-compliance-cli";
    });
in
  pkgs.symlinkJoin {
    name = "rust-template-compliance-cli";
    paths = [unwrapped];
    # makeWrapper provides wrapProgram, used below to bake `just` onto PATH.
    nativeBuildInputs = [pkgs.makeWrapper];
    # `just` is the recipe runner the justfile-recipe compliance check invokes
    # via `just --summary`; bundling it keeps the shipped CLI self-sufficient
    # instead of relying on an ambient `just`.
    #
    # The wrap is conditional because this override is applied to every build
    # variant (the release cross builds included), and a cross-compiled output
    # does not carry the native binary name — a Windows build ships
    # `rust-template-compliance-cli.exe`, which a Unix PATH wrapper could not
    # help anyway.  Those variants pass through unwrapped; the wrapped
    # guarantee is for the native package the checker actually runs.
    postBuild = ''
      if [ -x $out/bin/rust-template-compliance-cli ]; then
        wrapProgram $out/bin/rust-template-compliance-cli \
          --prefix PATH : ${pkgs.lib.makeBinPath [pkgs.just]}
      fi
    '';
  }
