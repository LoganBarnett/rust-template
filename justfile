# Run the entire test suite.
#
# This is the single "run everything" entry point invoked by both
# developers (locally) and CI.  Add new test scripts to this recipe
# so they are picked up automatically by both environments.
test: test-integration test-formatters

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
