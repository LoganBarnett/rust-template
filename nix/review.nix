# A spawn's justfile cannot call the review binary by name: new-project.sh
# rewrites every `rust-template` literal in the emitted files to the project
# name, so `rust-template-review-cli` would be emitted mangled.  This wrapper
# gives the binary a name free of that literal, which the emitted justfile's
# `review` recipe runs.  See crates/review-lib for what it does.
#
# No runtimeInputs: the tool reads the repository in-process (gix, not the
# git binary), and the reviewer it drives is the contributor's own `claude`
# install, which is not packaged and is found on the caller's PATH.
{
  writeShellApplication,
  # The compiled rust-template-review-cli binary this wrapper execs.
  review-cli,
}:
writeShellApplication {
  name = "review";
  text = ''
    exec ${review-cli}/bin/rust-template-review-cli "$@"
  '';
}
