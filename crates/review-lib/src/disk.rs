//! Reads of the working tree that see a path the way git does.
//!
//! Git never reads through a symlink: the link is a blob whose content is the
//! path it points at.  Every read of a change-set path goes through here so
//! the record, the packet, and the diff agree on what a path holds, and so a
//! link into a directory is a one-line file rather than a read that fails.

use gix::objs::tree::EntryKind;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What is at a path, as git would store it.
#[derive(Debug)]
pub enum Content {
  /// Nothing is there.
  Missing,
  /// A symlink; its content is the target it points at.
  Link(PathBuf),
  /// A regular file's bytes.
  File(Vec<u8>),
}

impl Content {
  /// The bytes git would hash for this content, or `None` when nothing is
  /// there.
  pub fn into_bytes(self) -> Option<Vec<u8>> {
    match self {
      Self::Missing => None,
      Self::Link(target) => Some(target.into_os_string().into_encoded_bytes()),
      Self::File(bytes) => Some(bytes),
    }
  }
}

/// The content at `path` below `root`.  A missing path is a kind of content
/// rather than an error; any other failure to read is returned, so the caller
/// decides whether the pass goes on without the path.
pub fn read(root: &Path, path: &str) -> io::Result<Content> {
  let full = root.join(path);
  fs::symlink_metadata(&full)
    .and_then(|metadata| {
      if metadata.file_type().is_symlink() {
        fs::read_link(&full).map(Content::Link)
      } else {
        fs::read(&full).map(Content::File)
      }
    })
    .or_else(or_missing(Content::Missing))
}

/// The entry kind at `path`, which the diff pipeline needs in order to load
/// the working-tree side the way git would: a link's target rather than
/// whatever it points at.  A path that is not there reads as a blob, which the
/// pipeline then finds missing.
pub fn kind(root: &Path, path: &str) -> io::Result<EntryKind> {
  fs::symlink_metadata(root.join(path))
    .map(|metadata| {
      if metadata.file_type().is_symlink() {
        EntryKind::Link
      } else {
        EntryKind::Blob
      }
    })
    .or_else(or_missing(EntryKind::Blob))
}

/// A recovery that reads an absent path as `missing` and passes every other
/// failure through.
fn or_missing<T>(missing: T) -> impl FnOnce(io::Error) -> io::Result<T> {
  |error| {
    if error.kind() == io::ErrorKind::NotFound {
      Ok(missing)
    } else {
      Err(error)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_missing_path_is_content_of_its_own_kind() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(read(dir.path(), "absent").unwrap(), Content::Missing));
    assert_eq!(kind(dir.path(), "absent").unwrap(), EntryKind::Blob);
  }

  #[cfg(unix)]
  #[test]
  fn a_link_into_a_directory_reads_as_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join("link")).unwrap();
    assert_eq!(
      read(dir.path(), "link").unwrap().into_bytes(),
      Some(target.into_os_string().into_encoded_bytes()),
    );
    assert_eq!(kind(dir.path(), "link").unwrap(), EntryKind::Link);
  }
}
