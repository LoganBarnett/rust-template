//! The working tree's difference from a tree, rendered as unified diff text
//! for the reviewer packet.
//!
//! `gix` loads each side and computes each hunk; the file headers are written
//! here, in git's shape, because a git library has no reason to privilege one
//! presentation of a change.  The reader is a reviewer, not `git apply`, so
//! a header carries what a reader needs — which file, and whether it is new,
//! gone, or renamed — and no `index` line.

use crate::error::{GitFailure, ReviewError};
use crate::worktree::Worktree;
use gix::bstr::{BStr, BString, ByteSlice};
use gix::diff::blob::pipeline::{Mode, WorktreeRoots};
use gix::diff::blob::platform::prepare_diff::{self, Operation, Outcome};
use gix::diff::blob::platform::resource::Data;
use gix::diff::blob::unified_diff::{ConsumeBinaryHunk, ContextSize};
use gix::diff::blob::{Algorithm, Platform, ResourceKind, UnifiedDiff};
use gix::objs::tree::EntryKind;
use gix::ObjectId;
use std::collections::BTreeMap;
use tracing::debug;

/// The lines of context around each hunk, as `git diff` prints by default.
const CONTEXT: u32 = 3;

/// The working tree's diff against `base`, restricted to `paths`, staged and
/// unstaged together.  A path in `origins` is diffed against the path it was
/// renamed from.
///
/// An empty `paths` is an empty diff: the packet restricts the reviewer to
/// what this round judges, and nothing is nothing.
pub fn render(
  worktree: &Worktree,
  base: &ObjectId,
  paths: &[String],
  origins: &BTreeMap<String, String>,
) -> Result<String, ReviewError> {
  if paths.is_empty() {
    Ok(String::new())
  } else {
    patches(worktree, base, paths, origins).map_err(ReviewError::PacketDiff)
  }
}

fn patches(
  worktree: &Worktree,
  base: &ObjectId,
  paths: &[String],
  origins: &BTreeMap<String, String>,
) -> Result<String, GitFailure> {
  let tree = worktree.tree_at(base)?;
  // The old side always comes from the object store; the new side is read
  // from the working tree, through the same filters git applies on the way
  // in, or is found missing there.
  let mut cache = worktree.repo().diff_resource_cache(
    Mode::ToGitUnlessBinaryToTextIsPresent,
    WorktreeRoots {
      old_root: None,
      new_root: Some(worktree.root().to_path_buf()),
    },
  )?;
  paths
    .iter()
    .map(|path| {
      file_patch(
        worktree,
        &tree,
        &mut cache,
        path,
        origins.get(path).map(String::as_str),
      )
    })
    .collect()
}

/// One file's patch, or nothing when its content did not move.
fn file_patch(
  worktree: &Worktree,
  tree: &gix::Tree<'_>,
  cache: &mut Platform,
  path: &str,
  origin: Option<&str>,
) -> Result<String, GitFailure> {
  let repo = worktree.repo();
  let old_path = origin.unwrap_or(path);
  let old = tree.lookup_entry_by_path(old_path)?;
  if old.as_ref().is_some_and(|entry| entry.mode().is_commit()) {
    debug!(path, "a submodule pointer has no patch to show");
    Ok(String::new())
  } else {
    let hash = repo.object_hash();
    cache.set_resource(
      old
        .as_ref()
        .map_or_else(|| ObjectId::null(hash), |entry| entry.object_id()),
      old
        .as_ref()
        .map_or(EntryKind::Blob, |entry| entry.mode().kind()),
      BStr::new(old_path),
      ResourceKind::OldOrSource,
      &repo.objects,
    )?;
    cache.set_resource(
      ObjectId::null(hash),
      EntryKind::Blob,
      BStr::new(path),
      ResourceKind::NewOrDestination,
      &repo.objects,
    )?;
    match cache.prepare_diff() {
      Ok(outcome) => rendered(repo, &outcome, path, origin),
      // A path the change set named that is gone from both sides by the
      // time it is rendered has nothing to show, which is not a failure.
      Err(prepare_diff::Error::SourceAndDestinationRemoved) => {
        Ok(String::new())
      }
      Err(source) => Err(GitFailure::from(source)),
    }
  }
}

fn rendered(
  repo: &gix::Repository,
  outcome: &Outcome<'_>,
  path: &str,
  origin: Option<&str>,
) -> Result<String, GitFailure> {
  let shape = Shape::of(outcome, path, origin);
  match outcome.operation {
    Operation::SourceOrDestinationIsBinary => Ok(format!(
      "{}Binary files {} and {} differ\n",
      shape.header(),
      shape.old_label(),
      shape.new_label()
    )),
    Operation::InternalDiff { algorithm } => {
      with_hunks(&shape, outcome, algorithm)
    }
    // An external diff driver is git's presentation choice, not ours; the
    // sides are already loaded, so the internal diff serves.
    Operation::ExternalCommand { .. } => {
      with_hunks(&shape, outcome, repo.diff_algorithm()?)
    }
  }
}

/// The header and hunks together, or nothing for a plain modification whose
/// content turns out not to differ.  An addition, deletion, or rename keeps
/// its header even with no hunks, as git does, since the header is the
/// change.
fn with_hunks(
  shape: &Shape<'_>,
  outcome: &Outcome<'_>,
  algorithm: Algorithm,
) -> Result<String, GitFailure> {
  let hunks = hunks(outcome, algorithm)?;
  Ok(if hunks.is_empty() && matches!(shape, Shape::Modified { .. }) {
    String::new()
  } else {
    format!("{}{hunks}", shape.header())
  })
}

fn hunks(
  outcome: &Outcome<'_>,
  algorithm: Algorithm,
) -> Result<String, GitFailure> {
  let input = outcome.interned_input();
  let diff = gix::diff::blob::diff_with_slider_heuristics(algorithm, &input);
  UnifiedDiff::new(
    &diff,
    &input,
    // Hunks accumulate as bytes so a file that is text but not UTF-8 still
    // renders rather than failing the packet.
    ConsumeBinaryHunk::new(BString::default(), "\n"),
    ContextSize::symmetrical(CONTEXT),
  )
  .consume()
  .map(|hunks| hunks.to_str_lossy().into_owned())
  .map_err(GitFailure::from)
}

/// What kind of change a file's patch describes, which is what its header
/// says.
#[derive(Debug, PartialEq, Eq)]
enum Shape<'a> {
  Added { path: &'a str, mode: EntryKind },
  Deleted { path: &'a str, mode: EntryKind },
  Modified { path: &'a str },
  Renamed { from: &'a str, to: &'a str },
}

impl<'a> Shape<'a> {
  fn of(outcome: &Outcome<'_>, path: &'a str, origin: Option<&'a str>) -> Self {
    let old_present = !matches!(outcome.old.data, Data::Missing);
    let new_present = !matches!(outcome.new.data, Data::Missing);
    match (origin, old_present, new_present) {
      (Some(from), _, _) => Self::Renamed { from, to: path },
      (None, false, _) => Self::Added {
        path,
        mode: outcome.new.mode,
      },
      (None, true, false) => Self::Deleted {
        path,
        mode: outcome.old.mode,
      },
      (None, true, true) => Self::Modified { path },
    }
  }

  fn header(&self) -> String {
    match self {
      Self::Added { path, mode } => format!(
        "diff --git a/{path} b/{path}\nnew file mode {}\n--- /dev/null\n+++ \
         b/{path}\n",
        octal(*mode)
      ),
      Self::Deleted { path, mode } => format!(
        "diff --git a/{path} b/{path}\ndeleted file mode {}\n--- a/{path}\n+++ \
         /dev/null\n",
        octal(*mode)
      ),
      Self::Modified { path } => {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n")
      }
      Self::Renamed { from, to } => format!(
        "diff --git a/{from} b/{to}\nrename from {from}\nrename to {to}\n--- \
         a/{from}\n+++ b/{to}\n"
      ),
    }
  }

  fn old_label(&self) -> String {
    match self {
      Self::Added { .. } => "/dev/null".to_string(),
      Self::Deleted { path, .. } | Self::Modified { path } => {
        format!("a/{path}")
      }
      Self::Renamed { from, .. } => format!("a/{from}"),
    }
  }

  fn new_label(&self) -> String {
    match self {
      Self::Deleted { .. } => "/dev/null".to_string(),
      Self::Added { path, .. } | Self::Modified { path } => format!("b/{path}"),
      Self::Renamed { to, .. } => format!("b/{to}"),
    }
  }
}

/// A file mode as git prints it in a patch header.
fn octal(mode: EntryKind) -> &'static str {
  match mode {
    EntryKind::BlobExecutable => "100755",
    EntryKind::Link => "120000",
    EntryKind::Commit => "160000",
    EntryKind::Tree => "040000",
    EntryKind::Blob => "100644",
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_addition_comes_from_nowhere() {
    let header = Shape::Added {
      path: "src/new.rs",
      mode: EntryKind::Blob,
    }
    .header();
    assert!(header.starts_with("diff --git a/src/new.rs b/src/new.rs\n"));
    assert!(header.contains("new file mode 100644\n"));
    assert!(header.contains("--- /dev/null\n+++ b/src/new.rs\n"));
  }

  #[test]
  fn a_deletion_goes_nowhere() {
    let header = Shape::Deleted {
      path: "bin/run",
      mode: EntryKind::BlobExecutable,
    }
    .header();
    assert!(header.contains("deleted file mode 100755\n"));
    assert!(header.contains("--- a/bin/run\n+++ /dev/null\n"));
  }

  #[test]
  fn a_modification_names_both_sides() {
    assert_eq!(
      Shape::Modified { path: "a.rs" }.header(),
      "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n",
    );
  }

  #[test]
  fn a_rename_names_where_it_came_from() {
    let header = Shape::Renamed {
      from: "old.rs",
      to: "new.rs",
    }
    .header();
    assert!(header.starts_with("diff --git a/old.rs b/new.rs\n"));
    assert!(header.contains("rename from old.rs\nrename to new.rs\n"));
    assert!(header.contains("--- a/old.rs\n+++ b/new.rs\n"));
  }

  #[test]
  fn binary_labels_follow_the_shape() {
    let added = Shape::Added {
      path: "logo.png",
      mode: EntryKind::Blob,
    };
    assert_eq!(added.old_label(), "/dev/null");
    assert_eq!(added.new_label(), "b/logo.png");
    let gone = Shape::Deleted {
      path: "logo.png",
      mode: EntryKind::Blob,
    };
    assert_eq!(gone.old_label(), "a/logo.png");
    assert_eq!(gone.new_label(), "/dev/null");
  }
}
