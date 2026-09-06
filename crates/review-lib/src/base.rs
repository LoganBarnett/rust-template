//! What the review compares against.

use crate::error::ReviewError;
use crate::worktree::{ChangeSet, Worktree};
use gix::ObjectId;
use std::path::Path;
use tracing::debug;

/// A resolved comparison, never `Auto` — resolution is what turns the request
/// into one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
  /// The working tree against `HEAD`: work in progress.
  Head,
  /// The working tree against where this branch left the default branch:
  /// everything the branch would land, committed or not.
  Branch,
}

impl DiffMode {
  fn label(self) -> &'static str {
    match self {
      Self::Head => "head",
      Self::Branch => "branch",
    }
  }
}

/// The comparison a review runs against, and the object it diffs from.
#[derive(Debug, Clone)]
pub struct Base {
  pub mode: DiffMode,
  /// The object the working tree is compared against: a commit, or the empty
  /// tree when there is no commit yet.
  pub commit: ObjectId,
  /// Whether the mode was chosen here rather than asked for, so the caller can
  /// say so — a scope the reader cannot see is a scope they will misread.
  pub inferred: bool,
}

impl Base {
  /// The review record's bucket name.  The mode is part of it because a
  /// branch diff shows strictly more of a file than a head diff does, so a
  /// verdict from one must never satisfy a request for the other.
  pub fn key(&self) -> String {
    format!("{}:{}", self.mode.label(), self.commit)
  }

  /// One line naming the scope, for a human and for the remediation prompt.
  pub fn describe(&self) -> String {
    let how = if self.inferred { " (inferred)" } else { "" };
    match self.mode {
      DiffMode::Head => {
        format!("working tree against HEAD{how}")
      }
      DiffMode::Branch => {
        format!("branch against {}{how}", short(&self.commit))
      }
    }
  }

  /// Every path this comparison covers.  With a fixture in place a head
  /// comparison reads its change set from it instead, so tests can drive a
  /// pass without a real tree.
  pub fn changed(
    &self,
    worktree: &Worktree,
    fixture: Option<&Path>,
  ) -> Result<ChangeSet, ReviewError> {
    match (self.mode, fixture) {
      (DiffMode::Head, Some(path)) => Worktree::changes_from_fixture(path),
      _ => worktree.changed(&self.commit),
    }
  }
}

/// The leading twelve hex digits, enough to name a commit to a reader.
fn short(commit: &ObjectId) -> String {
  commit.to_hex_with_len(12).to_string()
}

/// Resolve the requested comparison, where `None` means "decide from the
/// tree".  A repository with no commits has neither a `HEAD` to diff against
/// nor a branch to have left, so it always resolves to the empty tree.
pub fn resolve(
  worktree: &Worktree,
  requested: Option<DiffMode>,
) -> Result<Base, ReviewError> {
  worktree.head_commit()?.map_or_else(
    || {
      debug!("no commit yet; reviewing everything against the empty tree");
      Ok(Base {
        mode: DiffMode::Head,
        commit: worktree.empty_tree(),
        inferred: requested.is_none(),
      })
    },
    |head| {
      let inferred = requested.is_none();
      requested
        .map_or_else(|| infer(worktree, &head), Ok)
        .and_then(|mode| base_for(worktree, mode, head, inferred))
    },
  )
}

/// Uncommitted work means the tree is what you are working on; a clean tree
/// means the branch is.
fn infer(
  worktree: &Worktree,
  head: &ObjectId,
) -> Result<DiffMode, ReviewError> {
  worktree.is_dirty(head).map(|dirty| {
    if dirty {
      DiffMode::Head
    } else {
      DiffMode::Branch
    }
  })
}

fn base_for(
  worktree: &Worktree,
  mode: DiffMode,
  head: ObjectId,
  inferred: bool,
) -> Result<Base, ReviewError> {
  match mode {
    DiffMode::Head => Ok(head),
    DiffMode::Branch => worktree
      .default_branch()
      .and_then(|branch| worktree.merge_base(&branch)),
  }
  .map(|commit| Base {
    mode,
    commit,
    inferred,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn commit() -> ObjectId {
    ObjectId::empty_tree(gix::hash::Kind::Sha1)
  }

  #[test]
  fn the_key_separates_the_two_modes() {
    let head = Base {
      mode: DiffMode::Head,
      commit: commit(),
      inferred: false,
    };
    let branch = Base {
      mode: DiffMode::Branch,
      commit: commit(),
      inferred: false,
    };
    assert_ne!(
      head.key(),
      branch.key(),
      "a head verdict must not satisfy a branch request",
    );
  }

  #[test]
  fn an_inferred_scope_says_so() {
    assert!(Base {
      mode: DiffMode::Head,
      commit: commit(),
      inferred: true,
    }
    .describe()
    .contains("inferred"));
  }

  #[test]
  fn a_branch_scope_names_the_commit_briefly() {
    let described = Base {
      mode: DiffMode::Branch,
      commit: commit(),
      inferred: false,
    }
    .describe();
    assert!(described.contains(&commit().to_string()[..12]));
    assert!(!described.contains(&commit().to_string()));
  }
}
