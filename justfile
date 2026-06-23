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

# Code-review Stop hook unit test.
#
# Drives template/.claude/hooks/review-stop.sh through every release and
# block branch with crafted transcripts and a faked working-tree file
# list, guarding in particular the "Agent"-named subagent detection whose
# regression once wedged a real session.
test-review-stop:
  ./test-review-stop.sh

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
