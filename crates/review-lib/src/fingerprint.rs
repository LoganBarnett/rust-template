//! Content hashes for the paths under review.
//!
//! The record keys each file on the hash of its current content, so a file the
//! reviewer has already judged is recognised whether or not it has been
//! staged, and a file that merely moved in the change set is not re-reviewed.

use crate::error::{GitFailure, ReviewError};
use crate::worktree::Worktree;
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

/// The blob hash of each path's current content, `None` for a path that no
/// longer exists.  Paths are repository-relative, as the change set lists
/// them, and are resolved against the working tree's root so the result does
/// not depend on which directory the tool was run from.
pub fn blobs(
  worktree: &Worktree,
  paths: &[String],
) -> Result<BTreeMap<String, Option<String>>, ReviewError> {
  paths
    .iter()
    .map(|path| blob(worktree, path).map(|blob| (path.clone(), blob)))
    .collect()
}

/// The file's bytes hashed as git would store them as a blob.  The bytes are
/// taken as they are on disk, with no clean filter or line-ending conversion:
/// the record only ever compares these hashes against its own earlier values,
/// so agreement with git's filtered blob id is not needed.
fn blob(
  worktree: &Worktree,
  path: &str,
) -> Result<Option<String>, ReviewError> {
  match fs::read(worktree.root().join(path)) {
    Ok(bytes) => gix::objs::compute_hash(
      worktree.object_hash(),
      gix::objs::Kind::Blob,
      &bytes,
    )
    .map(|id| Some(id.to_string()))
    .map_err(|source| ReviewError::Fingerprint(GitFailure::from(source))),
    Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
    Err(source) => Err(ReviewError::FingerprintRead {
      path: PathBuf::from(path),
      source,
    }),
  }
}
