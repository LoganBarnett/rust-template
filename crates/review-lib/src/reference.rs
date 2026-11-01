//! A reference to the working tree.  The packet carries it in place of
//! content too large to inline.
//!
//! The reviewer can read the working tree, so an oversize file goes by
//! reference and the packet stays within the prompt.  The patch and the
//! untracked-file blocks share this so they say the same thing.

use std::fmt;

/// The limit `bytes` is over, if any.
pub fn exceeded(limit: Option<usize>, bytes: usize) -> Option<usize> {
  limit.filter(|limit| bytes > *limit)
}

/// Where the reviewer reads what the packet left behind.
#[derive(Debug)]
pub struct Reference<'a> {
  /// What the note opens with: `Patch` or `Content`.
  pub what: &'static str,
  /// The working-tree path to read, or `None` for a deleted file.
  pub path: Option<&'a str>,
  /// The size of what stayed behind.
  pub bytes: usize,
  /// The limit it crossed.
  pub limit: usize,
}

impl fmt::Display for Reference<'_> {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let read = self.path.map_or_else(
      || "The file is deleted, so there is nothing to read.".to_string(),
      |path| format!("Read {path} from the working tree with the Read tool."),
    );
    write!(
      f,
      "{} by reference: {} bytes exceeds --inline-max-bytes {}.  {read}",
      self.what, self.bytes, self.limit
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_limit_is_inclusive() {
    assert_eq!(exceeded(Some(10), 10), None);
    assert_eq!(exceeded(Some(10), 11), Some(10));
  }

  #[test]
  fn no_limit_references_nothing() {
    assert_eq!(exceeded(None, usize::MAX), None);
  }

  #[test]
  fn the_note_names_where_to_read() {
    assert_eq!(
      Reference {
        what: "Patch",
        path: Some("src/big.rs"),
        bytes: 2048,
        limit: 1024,
      }
      .to_string(),
      "Patch by reference: 2048 bytes exceeds --inline-max-bytes 1024.  Read \
       src/big.rs from the working tree with the Read tool."
    );
  }

  #[test]
  fn a_deleted_file_has_nothing_to_read() {
    assert!(Reference {
      what: "Patch",
      path: None,
      bytes: 2048,
      limit: 1024,
    }
    .to_string()
    .ends_with("The file is deleted, so there is nothing to read."));
  }
}
