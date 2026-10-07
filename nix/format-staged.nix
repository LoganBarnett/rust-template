# A spawn's pre-commit hook cannot call the format-staged binary by name.
# new-project.sh rewrites every `rust-template` literal in the emitted files to
# the project name.  `rust-template-format-staged-cli` would be emitted
# mangled.  This wrapper gives the binary a name free of that literal.  The
# emitted hook runs that name.  See crates/format-staged-lib for what it does.
{
  writeShellApplication,
  # The compiled rust-template-format-staged-cli binary this wrapper execs.
  format-staged-cli,
  # The tool the engine shells out to.  It reads the project's treefmt.toml
  # and dispatches each staged file to its formatter.
  treefmt,
}:
writeShellApplication {
  name = "format-staged";
  runtimeInputs = [treefmt];
  text = ''
    exec ${format-staged-cli}/bin/rust-template-format-staged-cli "$@"
  '';
}
