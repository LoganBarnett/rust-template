//! The index, written out as files a formatter can read.

use crate::FormatStagedError;
use gix::progress::Discard;
use gix::worktree::stack::state::attributes::Source;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use tempfile::TempDir;

/// The whole index checked out into a scratch directory, which is removed
/// when this is dropped.
pub(crate) struct Export {
  /// Owns the directory.
  _dir: TempDir,
  root: PathBuf,
}

impl Export {
  /// The directory that stands in for the repository root.
  pub(crate) fn root(&self) -> &Path {
    &self.root
  }
}

/// Check the whole index out into a scratch directory.
///
/// Every entry is exported, not only the staged ones.  A formatter reads its
/// configuration and a file's siblings relative to the file.  rustfmt needs
/// `rustfmt.toml` and every out-of-line module beside the file it formats.
pub(crate) fn export_index(
  repo: &gix::Repository,
  index: &gix::index::State,
) -> Result<Export, FormatStagedError> {
  let dir = tempfile::tempdir().map_err(FormatStagedError::ExportDirCreate)?;
  // treefmt rejects a file that is not under its tree root, and it compares
  // the two as text.  A temporary directory is often reached through a
  // symlink, so one resolved spelling is used for both.
  let root = dir.path().canonicalize().map_err(|source| {
    FormatStagedError::ExportDirResolve {
      dir: dir.path().to_path_buf(),
      source,
    }
  })?;
  let options = repo
    .checkout_options(Source::IdMapping)
    .map_err(|source| FormatStagedError::ExportOptions(Box::new(source)))
    .map(|options| gix::worktree::state::checkout::Options {
      destination_is_initially_empty: true,
      ..options
    })?;
  let objects = repo
    .objects
    .clone()
    .into_arc()
    .map_err(FormatStagedError::ExportObjects)?;
  // The checkout records each exported file's stat data in the index it is
  // handed.  A copy takes that, so the index written back never describes the
  // export.
  let mut scratch = index.clone();
  let outcome = gix::worktree::state::checkout(
    &mut scratch,
    &root,
    objects,
    &Discard,
    &Discard,
    &AtomicBool::new(false),
    options,
  )
  .map_err(|source| FormatStagedError::Export {
    dir: root.clone(),
    source: Box::new(source),
  })?;
  // On a case-insensitive filesystem two index paths can land on one file.
  // Its content would then be formatted and staged under the wrong path.
  if outcome.collisions.is_empty() {
    Ok(Export { _dir: dir, root })
  } else {
    Err(FormatStagedError::ExportCollision {
      paths: outcome
        .collisions
        .iter()
        .map(|collision| collision.path.to_string())
        .collect(),
    })
  }
}
