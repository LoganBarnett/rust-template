//! Content hashes for the paths under review.
//!
//! The record keys each file on the hash of its current content, so a file the
//! reviewer has already judged is recognised whether or not it has been
//! staged, and a file that merely moved in the change set is not re-reviewed.

use crate::disk::{self, Content};
use crate::error::{GitFailure, ReviewError};
use crate::worktree::Worktree;
use std::collections::BTreeMap;

/// What hashing the change set produced.
#[derive(Debug, Default)]
pub struct Blobs {
  /// The blob hash of each path's current content, `None` for a path that no
  /// longer exists.  Paths are repository-relative, as the change set lists
  /// them.
  pub hashes: BTreeMap<String, Option<String>>,
  /// The paths whose content could not be read, and so have no hash.
  pub unreadable: Vec<Unreadable>,
}

/// A path the pass could not read.
#[derive(Debug)]
pub struct Unreadable {
  pub path: String,
  pub source: std::io::Error,
}

/// One path's hash, or the reason it has none.
type Hashed = Result<(String, Option<String>), Unreadable>;

/// Hash each path's current content, resolved against the working tree's root
/// so the result does not depend on which directory the tool was run from.  A
/// path that cannot be read is set aside rather than failing the pass: it is
/// the caller's to report, and nothing else in the change set is any less
/// reviewable for it.
pub fn blobs(
  worktree: &Worktree,
  paths: &[String],
) -> Result<Blobs, ReviewError> {
  let (readable, unreadable): (Vec<Hashed>, Vec<Hashed>) = paths
    .iter()
    .map(|path| hashed(worktree, path))
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    .partition(Result::is_ok);
  Ok(Blobs {
    hashes: readable.into_iter().flatten().collect(),
    unreadable: unreadable.into_iter().filter_map(Result::err).collect(),
  })
}

/// One path read and hashed.  The outer failure is the hash itself failing,
/// which does fail the pass; an unreadable path is the inner, expected case.
fn hashed(worktree: &Worktree, path: &str) -> Result<Hashed, ReviewError> {
  disk::read(worktree.root(), path).map_or_else(
    |source| {
      Ok(Err(Unreadable {
        path: path.to_string(),
        source,
      }))
    },
    |content| hash(worktree, content).map(|blob| Ok((path.to_string(), blob))),
  )
}

/// The content hashed as git would store it as a blob.  The bytes are taken
/// as they are on disk, with no clean filter or line-ending conversion: the
/// record only ever compares these hashes against its own earlier values, so
/// agreement with git's filtered blob id is not needed.
fn hash(
  worktree: &Worktree,
  content: Content,
) -> Result<Option<String>, ReviewError> {
  content
    .into_bytes()
    .map(|bytes| {
      gix::objs::compute_hash(
        worktree.object_hash(),
        gix::objs::Kind::Blob,
        &bytes,
      )
      .map(|id| id.to_string())
      .map_err(|source| ReviewError::Fingerprint(GitFailure::from(source)))
    })
    .transpose()
}
