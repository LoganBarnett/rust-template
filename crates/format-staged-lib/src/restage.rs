//! The formatted export, written back as blobs and index entries.

use crate::error::CleanFailure;
use crate::staged::StagedFile;
use crate::FormatStagedError;
use gix::index::entry::{Stage, Stat};
use std::io::Read;
use std::path::Path;
use tap::Tap;
use tracing::info;

/// A staged file whose formatted content differs from what was staged.
struct Reformatted<'a> {
  file: &'a StagedFile,
  /// The blob holding the formatted content.
  id: gix::ObjectId,
  /// Whether the working file held exactly the staged content, which means
  /// it carries no unstaged change that rewriting it could lose.
  working_file_in_sync: bool,
}

/// Stage the formatted content, then bring along every working file that had
/// no unstaged changes.
pub(crate) fn restage(
  repo: &gix::Repository,
  index: &mut gix::index::File,
  export: &Path,
  workdir: &Path,
  staged: &[StagedFile],
) -> Result<(), FormatStagedError> {
  let (mut filters, _) = repo
    .filter_pipeline(None)
    .map_err(FormatStagedError::FilterPipeline)?;
  let changed = staged
    .iter()
    .map(|file| reformatted(repo, &mut filters, index, export, workdir, file))
    .filter_map(Result::transpose)
    .collect::<Result<Vec<_>, _>>()?;
  // An untouched index keeps the extensions a rewrite would drop, so nothing
  // is written when formatting changed nothing.
  if changed.is_empty() {
    Ok(())
  } else {
    changed
      .iter()
      .try_for_each(|change| replace_blob(index, change))?;
    // gix writes the cache tree back as it read it.  A tree recorded before
    // this run would let the commit reuse the unformatted blobs.
    index.remove_tree();
    index
      .write(gix::index::write::Options::default())
      .map_err(|source| FormatStagedError::IndexWrite {
        path: index.path().to_path_buf(),
        source: gix::Error::from(source),
      })?;
    changed
      .iter()
      .tap(|_| info!(count = changed.len(), "re-staged formatted content"))
      .filter(|change| change.working_file_in_sync)
      .try_for_each(|change| sync_working_file(export, workdir, change.file))
  }
}

/// The formatted content of one staged file, when formatting changed it.
fn reformatted<'a>(
  repo: &gix::Repository,
  filters: &mut gix::filter::Pipeline<'_>,
  index: &gix::index::State,
  export: &Path,
  workdir: &Path,
  file: &'a StagedFile,
) -> Result<Option<Reformatted<'a>>, FormatStagedError> {
  let id = cleaned(filters, index, &export.join(&file.path), &file.path)
    .map_err(|source| FormatStagedError::FormattedContentClean {
      path: file.path.clone(),
      source,
    })
    .and_then(|content| {
      repo
        .write_blob(content)
        .map(gix::Id::detach)
        .map_err(|source| FormatStagedError::FormattedBlobWrite {
          path: file.path.clone(),
          source,
        })
    })?;
  if id == file.id {
    Ok(None)
  } else {
    working_file_in_sync(repo, filters, index, workdir, file).map(
      |working_file_in_sync| {
        Some(Reformatted {
          file,
          id,
          working_file_in_sync,
        })
      },
    )
  }
}

/// Whether the working file holds exactly the content that was staged.
fn working_file_in_sync(
  repo: &gix::Repository,
  filters: &mut gix::filter::Pipeline<'_>,
  index: &gix::index::State,
  workdir: &Path,
  file: &StagedFile,
) -> Result<bool, FormatStagedError> {
  let path = workdir.join(&file.path);
  // A working path that is missing, or is no longer a regular file, differs
  // from the staged blob by definition.
  let is_file = std::fs::symlink_metadata(&path)
    .map(|metadata| metadata.is_file())
    .or_else(|source| {
      if source.kind() == std::io::ErrorKind::NotFound {
        Ok(false)
      } else {
        Err(FormatStagedError::WorkingFileStat {
          path: file.path.clone(),
          source,
        })
      }
    })?;
  if is_file {
    cleaned(filters, index, &path, &file.path)
      .map_err(|source| FormatStagedError::WorkingFileClean {
        path: file.path.clone(),
        source,
      })
      .and_then(|content| {
        gix::objs::compute_hash(
          repo.object_hash(),
          gix::objs::Kind::Blob,
          &content,
        )
        .map_err(|source| FormatStagedError::WorkingFileHash {
          path: file.path.clone(),
          source: gix::Error::from(source),
        })
      })
      .map(|id| id == file.id)
  } else {
    Ok(false)
  }
}

/// The content of the file at `source` as git would store it for `rela_path`.
fn cleaned(
  filters: &mut gix::filter::Pipeline<'_>,
  index: &gix::index::State,
  source: &Path,
  rela_path: &Path,
) -> Result<Vec<u8>, CleanFailure> {
  let file = std::fs::File::open(source).map_err(CleanFailure::Open)?;
  let mut filtered = filters
    .convert_to_git(file, rela_path, index)
    .map_err(CleanFailure::Filter)?;
  let mut content = Vec::new();
  filtered
    .read_to_end(&mut content)
    .map(|_| content)
    .map_err(CleanFailure::Read)
}

/// Point an index entry at its formatted blob.
fn replace_blob(
  index: &mut gix::index::State,
  change: &Reformatted<'_>,
) -> Result<(), FormatStagedError> {
  index
    .entry_mut_by_path_and_stage(
      change.file.rela_path.as_ref(),
      Stage::Unconflicted,
    )
    .map(|entry| {
      entry.id = change.id;
      // The old stat data describes the unformatted working file and would
      // mark this entry clean against it.  Zeroed data makes git compare
      // content instead.
      entry.stat = Stat::default();
    })
    .ok_or_else(|| FormatStagedError::IndexEntryMissing {
      path: change.file.rela_path.to_string(),
    })
}

/// Give a working file the formatted content, so the tree matches the commit.
fn sync_working_file(
  export: &Path,
  workdir: &Path,
  file: &StagedFile,
) -> Result<(), FormatStagedError> {
  std::fs::copy(export.join(&file.path), workdir.join(&file.path))
    .map(|_| info!(path = %file.rela_path, "formatted the working file"))
    .map_err(|source| FormatStagedError::WorkingFileSync {
      path: file.path.clone(),
      source,
    })
}
