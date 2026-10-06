//! rust-template-format-staged-lib: the engine behind the pre-commit
//! formatter.
//!
//! A pre-commit hook has to format what is about to be committed, and that is
//! the index, not the working tree.  Formatting the working files and adding
//! them again stages every hunk the author left unstaged.  This engine exports
//! the index to a scratch directory, runs treefmt there, and writes the result
//! back as blobs.  A working file is rewritten only when it already matched
//! the index.

mod error;
mod export;
mod formatter;
mod restage;
mod staged;

pub use error::{CleanFailure, FormatStagedError};
pub use rust_template_foundation::logging::{LogFormat, LogLevel};

/// Format the staged content of the repository enclosing the current
/// directory.
///
/// The repository is found the way git finds it for a hook: `GIT_DIR` and
/// `GIT_INDEX_FILE` are honored.  `git commit --all` relies on the second,
/// because it hands the hook an index that is not the repository's own.
pub fn run() -> Result<(), FormatStagedError> {
  let repo = gix::discover_with_environment_overrides(".")
    .map_err(|source| FormatStagedError::RepositoryOpen(Box::new(source)))?;
  let workdir = repo
    .workdir()
    .ok_or(FormatStagedError::RepositoryBare)?
    .to_path_buf();
  let mut index = staged::rewritable_index(&repo)?;
  let staged = staged::staged_files(&repo, &index)?;
  if staged.is_empty() {
    Ok(())
  } else {
    let export = export::export_index(&repo, &index)?;
    formatter::format(export.root(), &staged)?;
    restage::restage(&repo, &mut index, export.root(), &workdir, &staged)
  }
}
