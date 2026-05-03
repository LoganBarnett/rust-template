#!/usr/bin/env bash
# Shared helpers for new-project.sh and crate-add.sh.

# Detect whether the sed in PATH is GNU (supports -i without an extension
# argument) or BSD (requires -i '').
if sed --version 2>/dev/null | grep -q GNU; then
    sed_inplace() { sed -i "$@"; }
else
    sed_inplace() { sed -i '' "$@"; }
fi
