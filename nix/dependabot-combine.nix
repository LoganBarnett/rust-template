# Packages dependabot-combine.sh as a shellcheck-linted command with every tool
# it invokes on PATH, so a contributor never needs gh, cargo, or the rest
# installed globally.  See dependabot-combine.sh for what it does.
{
  writeShellApplication,
  # GitHub CLI: lists the open bump PRs and their check status, opens the
  # combined PR, merges it, and closes the superseded PRs.
  gh,
  # Drives the throwaway worktree, replays each bump's manifest change, and
  # pushes the combined branch.
  git,
  # `cargo update --precise` reconciles Cargo.lock to Dependabot's chosen
  # versions without re-resolving anything; supplied as the project toolchain.
  rustToolchain,
  # Inserts the combined PR's changelog entries so CI's changelog gate passes.
  changelog-roller,
  # Resolves which git remote hosts the GitHub repo (the `awk` over
  # `git remote --verbose`).
  gawk,
  # Tests the combined PR's aggregated check output for pending/failing runs.
  gnugrep,
  # sleep, cat, rm, and the other core utilities the script relies on.
  coreutils,
}:
writeShellApplication {
  name = "dependabot-combine";
  runtimeInputs = [
    gh
    git
    rustToolchain
    changelog-roller
    gawk
    gnugrep
    coreutils
  ];
  text = builtins.readFile ./dependabot-combine.sh;
}
