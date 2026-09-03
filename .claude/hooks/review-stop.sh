#!/usr/bin/env bash
# Stop hook: the code-review gate, for the rust-template repository itself.
#
# Intentionally run differently than the template.  This ensures we get the same
# code that we see in the review repo, instead of something that would lag
# behind if we had a pinned flake.
exec cargo run --quiet --package rust-template-review-stop -- "$@"
