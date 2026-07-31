# Wraps the dependency-bump-cli crate binary with every external tool it
# shells out to on PATH, so a spawn (or the scheduled workflow) can `nix run`
# it with no environment assumptions.  See crates/dependency-bump-lib for
# what it does.
{
  writeShellApplication,
  # The compiled rust-template-dependency-bump-cli binary this wrapper execs.
  dependency-bump-cli,
  # `git diff` guards the clean-lockfile precondition — the post-update
  # lockfile diff is the bump report, so it must start clean.
  git,
  # `cargo update` advances every package to the newest release its existing
  # Cargo.toml constraint allows (and re-pins held packages with --precise);
  # `cargo audit` is a cargo subcommand, so cargo itself must be present.
  rustToolchain,
  # Classifies bumps: an advisory against the pre-update lockfile files the
  # bump under Security instead of Maintenance.
  cargo-audit,
  # Inserts the composed changelog entries so CI's changelog gate passes.
  changelog-roller,
  # Normalises the composed entries the way a local pre-commit treefmt would,
  # so the auto-committed changelog does not churn a later commit's diff.
  org-fmt,
}:
writeShellApplication {
  name = "dependency-bump";
  runtimeInputs = [
    git
    rustToolchain
    cargo-audit
    changelog-roller
    org-fmt
  ];
  text = ''
    exec ${dependency-bump-cli}/bin/rust-template-dependency-bump-cli "$@"
  '';
}
