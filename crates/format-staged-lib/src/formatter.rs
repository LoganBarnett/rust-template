//! The treefmt run over the exported index.

use crate::staged::StagedFile;
use crate::FormatStagedError;
use std::path::Path;
use std::process::Command;

/// Format the staged files where they sit in the export.
///
/// `treefmt` is found on `PATH`.  The packaged wrapper puts its own there.
/// The formatters treefmt dispatches to come from the caller's `PATH`.  They
/// are the project's own.
pub(crate) fn format(
  export: &Path,
  staged: &[StagedFile],
) -> Result<(), FormatStagedError> {
  Command::new("treefmt")
    // The cache is keyed by tree root, and this root is new on every run.
    .arg("--no-cache")
    // The export is not a git repository, so a git walk would find nothing.
    .args(["--walk", "filesystem"])
    .arg("--tree-root")
    .arg(export)
    .arg("--")
    .args(staged.iter().map(|file| &file.path))
    .current_dir(export)
    .status()
    .map_err(|source| FormatStagedError::FormatterInvocation { source })
    .and_then(|status| {
      status
        .success()
        .then_some(())
        .ok_or(FormatStagedError::FormatterFailed { status })
    })
}
