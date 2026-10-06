//! Which index entries a commit is about to change.

use crate::FormatStagedError;
use gix::bstr::BString;
use gix::index::entry::{Flags, Mode, Stage};
use std::borrow::Cow;
use std::path::PathBuf;

/// A staged regular file whose content or mode differs from `HEAD`.
pub(crate) struct StagedFile {
  /// The path as the index spells it, relative to the repository root.
  pub(crate) rela_path: BString,
  /// The same path in the platform's form.
  pub(crate) path: PathBuf,
  /// The blob the index held before formatting.
  pub(crate) id: gix::ObjectId,
}

/// The index a commit is being built from, owned so it can be rewritten.
pub(crate) fn rewritable_index(
  repo: &gix::Repository,
) -> Result<gix::index::File, FormatStagedError> {
  // gix folds a split index into one on read and cannot say afterwards that
  // it did, so the setting is the only place left to ask.
  if repo.config_snapshot().boolean("core.splitIndex") == Some(true) {
    Err(FormatStagedError::IndexSplit)
  } else {
    // A repository with nothing added yet has no index file.  That is an
    // empty index, not a failure.
    repo
      .index_or_empty()
      .map(|shared| gix::index::File::clone(&shared))
      .map_err(|source| FormatStagedError::IndexRead {
        path: repo.index_path(),
        source: Box::new(source),
      })
      .and_then(|index| {
        if index.is_sparse() {
          Err(FormatStagedError::IndexSparse {
            path: repo.index_path(),
          })
        } else {
          Ok(index)
        }
      })
  }
}

/// The staged regular files, in index order.
pub(crate) fn staged_files(
  repo: &gix::Repository,
  index: &gix::index::State,
) -> Result<Vec<StagedFile>, FormatStagedError> {
  // An unborn `HEAD` reads as the empty tree, so every entry of a first
  // commit counts as staged.
  let head = repo
    .head_tree_id_or_empty()
    .map_err(|source| FormatStagedError::HeadTreeResolve(Box::new(source)))
    .and_then(|tree| {
      repo
        .index_from_tree(&tree)
        .map_err(|source| FormatStagedError::HeadIndexBuild(Box::new(source)))
    })?;
  index
    .entries()
    .iter()
    .filter(|entry| is_formattable(entry))
    .filter(|entry| {
      head
        .entry_by_path_and_stage(entry.path(index), Stage::Unconflicted)
        .is_none_or(|committed| {
          (committed.id, committed.mode) != (entry.id, entry.mode)
        })
    })
    .map(|entry| {
      let rela_path = entry.path(index).to_owned();
      gix::path::try_from_bstr(&rela_path)
        .map(Cow::into_owned)
        .map_err(|source| FormatStagedError::StagedPathConvert {
          path: rela_path.to_string(),
          source,
        })
        .map(|path| StagedFile {
          rela_path,
          path,
          id: entry.id,
        })
    })
    .collect()
}

/// Whether an entry holds content a formatter can rewrite.  A symlink or a
/// submodule has none.  An intent-to-add entry has staged nothing yet.  A
/// skip-worktree entry is left out of the export, so there is no file to
/// format.
fn is_formattable(entry: &gix::index::Entry) -> bool {
  entry.stage() == Stage::Unconflicted
    && [Mode::FILE, Mode::FILE_EXECUTABLE].contains(&entry.mode)
    && !entry
      .flags
      .intersects(Flags::INTENT_TO_ADD | Flags::SKIP_WORKTREE)
}
