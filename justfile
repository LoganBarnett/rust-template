# -*- mode: just; just-indent-offset: 2; indent-tabs-mode: nil; -*-
# Run the entire test suite.
#
# The developer entry point: runs every test script locally.  CI runs the
# same scripts as a matrix in .github/workflows/ci.yml rather than through
# this recipe, so a new script must be added in both places to gate
# everywhere — here for local runs and there for CI.
test: test-integration test-formatters ci-test test-review-stop

# Crate-add and project-emission integration tests.
#
# Emits projects into temporary directories and asserts on file
# layout, sentinel markers, name substitution, and (when nix is
# available) flake-evaluation cleanliness.  The flake-eval check
# catches API drift between the template's emitted flake.nix and
# the foundation library it depends on.
test-integration:
  ./test-crate-add.sh

# Formatter-coverage test.
#
# Spawns a fresh project and verifies that every formatter declared
# in template/treefmt.toml is on PATH inside the devShell and
# actually rewrites known-malformed files.
test-formatters:
  ./test-formatters.sh

# Full-CI emission test.
#
# Emits a fresh cli+server project, points its foundation dependency at the
# on-disk rust-template, and runs the same clippy and test gates the spawn's
# own CI runs.  Catches archetype drift that only surfaces on a real compile —
# the other emission tests evaluate the flake but never build the crates.
ci-test:
  ./ci-test.sh

# Code-review Stop hook gate tests.
#
# Runs the review-stop crate's tests: the black-box suite drives the gate
# binary through every release and block branch with crafted transcripts
# and a faked working-tree file list.
test-review-stop *args:
  cargo test --package rust-template-review-stop {{args}}

# Audit every registered spawn against the compliance manifest.
#
# Runs the Rust compliance checker (crates/compliance-cli) against the
# spawns in config.json, reporting per-check results (each with a stable
# id) and exiting non-zero when any check fails.  This is a fleet audit,
# deliberately not part of `test`: the template repo's build must not gate
# on whether other repositories are currently in compliance.  Arguments
# pass through, e.g. `just compliance --project my-app --format json`.
compliance *ARGS:
  cargo run --quiet --package rust-template-compliance-cli -- {{ARGS}}

# Combine this repo's passing Dependabot PRs into one PR and merge it.
#
# One-shot catch-up for a Dependabot backlog.  Runs the dependabot-combine
# package this flake exposes (nix/dependabot-combine.*) — the same package
# spawns pull from foundation — put on PATH by this repo's dev shell.  The
# script auto-detects the github.com remote, so this repo's Gitea origin is a
# non-issue.  Pass --dry-run or --no-merge to hold back.
dependabot-combine *args:
  dependabot-combine {{args}}

# Bump every dependency the workspace's constraints allow, with changelog.
#
# The working-tree half of the scheduled dependency-bump flow: runs `cargo
# update` across the workspace, classifies each bump against `cargo audit`,
# and composes the CHANGELOG entries — then stops.  Nothing is committed or
# pushed; review the diff and commit yourself.  The scheduled workflow
# (.github/workflows/dependency-bump.yml) runs the same engine and owns the
# branch/PR/merge half.  Runs from source (like `just compliance`) so local
# changes to the crates are exercised directly.  Pass --dry-run true to
# preview.
dependency-bump *args:
  cargo run --quiet --package rust-template-dependency-bump-cli -- {{args}}
