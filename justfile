# Run the entire test suite.
#
# This is the single "run everything" entry point invoked by both
# developers (locally) and CI.  Add new test scripts to this recipe
# so they are picked up automatically by both environments.
test: test-integration test-formatters test-review-stop

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
