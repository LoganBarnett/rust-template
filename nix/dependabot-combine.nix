# Packages dependabot-combine.sh as a shellcheck-linted command with every tool
# it invokes on PATH, so a contributor never needs gh, cargo, or the rest
# installed globally.  See dependabot-combine.sh for what it does.
{
  writeShellApplication,
  # GitHub CLI: lists the open bump PRs and their check status, reads each PR's
  # diff, opens the combined PR, merges it, and closes the superseded PRs.
  gh,
  # Clones the target repo into the throwaway checkout, branches off the base,
  # and pushes the combined branch.
  git,
  # `cargo update` bumps each package a PR signalled to the newest release its
  # existing constraint allows; supplied as the project toolchain.
  rustToolchain,
  # Inserts the combined PR's changelog entries so CI's changelog gate passes.
  changelog-roller,
  # `awk` parses each Cargo.lock diff for the packages that moved and picks the
  # git remote that hosts the GitHub repo.
  gawk,
  # Tests the combined PR's aggregated check output for pending/failing runs.
  gnugrep,
  # mkdir, cut, rm, sleep, and the other core utilities the script relies on.
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
