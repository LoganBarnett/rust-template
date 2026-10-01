# The checker's justfile-recipe check shells out to `just --summary` at
# runtime, so the native compliance-cli package carries `just` on its PATH and
# works in any Nix context, not only a dev shell that happens to provide it.
#
# The wrap lives here, at the consumption site, and not in a
# `nix/packages/compliance-cli.nix` override: an override is applied to every
# build variant, the release cross builds included, and a wrapper script is not
# a release binary.  The wrapped package is what `packages.default` and
# `apps.compliance-cli` expose; the cross variants stay plain crane builds.
{
  lib,
  symlinkJoin,
  makeWrapper,
  just,
  compliance-cli,
}:
symlinkJoin {
  name = "rust-template-compliance-cli";
  paths = [compliance-cli];
  # makeWrapper provides wrapProgram, used below to bake `just` onto PATH.
  nativeBuildInputs = [makeWrapper];
  postBuild = ''
    wrapProgram $out/bin/rust-template-compliance-cli \
      --prefix PATH : ${lib.makeBinPath [just]}
  '';
}
