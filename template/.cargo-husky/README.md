# cargo-husky Pre-commit Hooks

This repository uses `cargo-husky` to automatically run `treefmt` before each commit.

## How it works

- `cargo-husky` is configured in the root `Cargo.toml` as a dev-dependency
- When you run any cargo command (like `cargo build`, `cargo test`, etc.), cargo-husky will automatically install the git hooks
- The hooks are defined in `.cargo-husky/hooks/pre-commit`

## Installation

The hooks will be automatically installed to `.git/hooks/` the first time you run any cargo command after cloning the repository:

```bash
cargo build
# or
cargo test
```

## What the pre-commit hook does

The hook runs `format-staged`. The dev shell provides it. Before each commit
it will:
1. Identify which files are staged for commit
2. Run `treefmt` on the staged content of those files, dispatching each to the
   formatter declared in `treefmt.toml`
3. Re-stage the formatted content
4. Proceed with the commit, ensuring all committed code is properly formatted

Note: Only staged content is formatted and included. A hunk you left unstaged
stays out of the commit. A file that carries unstaged changes is not rewritten
in your working directory. Its unstaged diff then reads as both your edit and
an undoing of the formatting. Nothing is lost. Stage the whole file and commit
again to settle it. A fully staged file is also updated in place so it matches
the commit.

Commit from inside the dev shell. Outside it the hook cannot find
`format-staged`, and the commit stops.

`format-staged` refuses a repository that uses a split index or a sparse
index. It cannot rewrite either one safely. The split index is the one
`core.splitIndex` turns on.

## Bypassing the hook (not recommended)

If you absolutely need to commit unformatted code, you can bypass the hook with:

```bash
git commit --no-verify
```

However, this is not recommended as it defeats the purpose of having the hook.
