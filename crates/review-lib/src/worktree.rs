//! Every read the review makes of the repository.
//!
//! The repository is read in-process through `gix` rather than by running
//! `git`: nothing here parses git's output, so a locale that translates git's
//! messages cannot change what the review concludes, and the tool still sees
//! the tree the way the contributor's own configuration presents it, since
//! `gix` reads that configuration too.

use crate::error::{GitFailure, ReviewError};
use gix::bstr::{BStr, BString, ByteSlice};
use gix::dir::entry::{Kind, Status};
use gix::refs::TargetRef;
use gix::status::index_worktree::Item;
use gix::status::UntrackedFiles;
use gix::worktree::IndexPersistedOrInMemory;
use gix::ObjectId;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

/// The review record, and the temporary file the atomic store renames from.
/// Both are filtered out of every listing this module makes, so the record is
/// never part of the change set it describes.  The `.gitignore` entry is the
/// ordinary mechanism; this is what holds when a repository lacks it.
pub const RECORD_PREFIX: &str = "review.json";

/// Where the remote's default branch is recorded locally.
const REMOTE_HEAD: &str = "refs/remotes/origin/HEAD";

/// The paths a comparison covers.
#[derive(Debug, Default, Clone)]
pub struct ChangeSet {
  /// Every path that differs between the base and the working tree,
  /// deduplicated and sorted.
  pub paths: Vec<String>,
  /// For a path that arrived by rename, the path it came from at the base,
  /// keyed by where it is now.  The origin itself is not in `paths`: the
  /// current path already names the change.
  pub origins: BTreeMap<String, String>,
}

/// The repository enclosing the current directory, opened once per pass.
pub struct Worktree {
  repo: gix::Repository,
  root: PathBuf,
}

impl Worktree {
  /// The repository enclosing the current directory, which must have a
  /// working tree.
  pub fn open() -> Result<Self, ReviewError> {
    Self::discover().map_err(ReviewError::RepositoryOpen)
  }

  fn discover() -> Result<Self, GitFailure> {
    let repo = gix::discover_with_environment_overrides(".")?;
    let root = repo.workdir().ok_or(GitFailure::Bare)?.to_path_buf();
    Ok(Self { repo, root })
  }

  /// The absolute path of the working tree's top-level directory, where the
  /// review record lives and against which every listed path resolves.
  pub fn root(&self) -> &Path {
    &self.root
  }

  pub(crate) fn repo(&self) -> &gix::Repository {
    &self.repo
  }

  /// The hash function the repository's object ids use.
  pub fn object_hash(&self) -> gix::hash::Kind {
    self.repo.object_hash()
  }

  /// Every path that differs between `base` and the working tree.  A commit
  /// moves a path out of the working tree's changes against `HEAD` but not
  /// out of its changes against an older base, which is what lets a
  /// branch-scoped review see work that is already committed.
  pub fn changed(&self, base: &ObjectId) -> Result<ChangeSet, ReviewError> {
    self
      .changed_between(base)
      .map(|set| without_record(set, "change set"))
      .map_err(ReviewError::WorkingTreeList)
  }

  /// A change set read from a fixture file instead of the tree, so tests can
  /// drive a pass without a real repository.
  pub fn changes_from_fixture(path: &Path) -> Result<ChangeSet, ReviewError> {
    fs::read_to_string(path)
      .map(|listing| ChangeSet {
        paths: non_empty_lines(&listing).collect(),
        origins: BTreeMap::new(),
      })
      .map(|set| without_record(set, "change set"))
      .map_err(|source| ReviewError::WorkingTreeFixtureRead {
        path: path.to_path_buf(),
        source,
      })
  }

  /// Whether the working tree differs from `head` at all, which is how an
  /// unspecified diff base decides between reviewing uncommitted work and
  /// reviewing the branch.  Untracked files count, as they do for `git
  /// status`.
  pub fn is_dirty(&self, head: &ObjectId) -> Result<bool, ReviewError> {
    self.changed(head).map(|set| !set.paths.is_empty())
  }

  /// The status walk between `base` and the working tree.  An index built
  /// from the base tree carries no stat data, so every file the tree has is
  /// compared by content, and everything else on disk that git does not
  /// ignore is walked as untracked — which is exactly what differs from the
  /// tree, and nothing that merely differs from the index.
  fn changed_between(&self, base: &ObjectId) -> Result<ChangeSet, GitFailure> {
    let index = self.repo.index_from_tree(&self.tree_at(base)?.id)?;
    self
      .repo
      .status(gix::progress::Discard)?
      .index(IndexPersistedOrInMemory::InMemory(index))
      .untracked_files(UntrackedFiles::Files)
      // Rename tracking is off unless asked for; git has no configuration
      // for renames between an index and the working tree, so the defaults
      // (renames only, at git's own similarity threshold) are the closest to
      // what `git status` shows.
      .index_worktree_rewrites(Some(gix::diff::Rewrites::default()))
      .into_index_worktree_iter(Vec::<BString>::new())?
      .try_fold(Accumulated::default(), |set, item| {
        item.map(|item| set.absorb(&item)).map_err(GitFailure::from)
      })
      .map(Accumulated::into_change_set)
  }

  /// The files present in the working tree that git does not track and does
  /// not ignore.
  pub fn untracked_files(&self) -> Result<Vec<String>, ReviewError> {
    self
      .untracked()
      .map(|paths| without_record_paths(paths, "untracked files"))
      .map_err(ReviewError::UntrackedList)
  }

  fn untracked(&self) -> Result<Vec<String>, GitFailure> {
    self
      .repo
      .status(gix::progress::Discard)?
      .untracked_files(UntrackedFiles::Files)
      .into_index_worktree_iter(Vec::<BString>::new())?
      .filter_map(|item| match item {
        Ok(Item::DirectoryContents { entry, .. })
          if is_untracked_file(&entry) =>
        {
          Some(Ok(text(entry.rela_path.as_bstr())))
        }
        Ok(_) => None,
        Err(source) => Some(Err(GitFailure::from(source))),
      })
      .collect()
  }

  /// The commit `HEAD` names, or `None` on an unborn branch (a repository with
  /// no commits yet).
  pub fn head_commit(&self) -> Result<Option<ObjectId>, ReviewError> {
    self.head().map_err(ReviewError::HeadResolve)
  }

  fn head(&self) -> Result<Option<ObjectId>, GitFailure> {
    Ok(
      self
        .repo
        .head()?
        .try_into_peeled_id()?
        .map(|id| id.detach()),
    )
  }

  /// The hash of the empty tree, the diff base that shows every file as added
  /// when there is no commit to diff against.
  pub fn empty_tree(&self) -> ObjectId {
    ObjectId::empty_tree(self.repo.object_hash())
  }

  /// The branch a pull request would target: what `origin/HEAD` points at, or
  /// a local `main` or `master` when the remote does not say.  A repository
  /// with none of the three cannot be branch-diffed, which the caller reports
  /// rather than guessing.
  pub fn default_branch(&self) -> Result<String, ReviewError> {
    let resolve = ReviewError::DefaultBranchResolve;
    self.remote_head().map_err(resolve)?.map_or_else(
      || {
        ["main", "master"]
          .into_iter()
          .map(|branch| {
            self
              .branch_exists(branch)
              .map(|exists| exists.then(|| branch.to_string()))
          })
          .collect::<Result<Vec<_>, _>>()
          .map_err(resolve)?
          .into_iter()
          .flatten()
          .next()
          .ok_or(ReviewError::DefaultBranchUnresolved)
      },
      Ok,
    )
  }

  /// What `origin/HEAD` points at, with the remote prefix stripped, or `None`
  /// when the remote's default is not recorded locally — a common state in a
  /// clone made without it, and not an error.
  fn remote_head(&self) -> Result<Option<String>, GitFailure> {
    Ok(
      self
        .repo
        .try_find_reference(REMOTE_HEAD)?
        .and_then(|reference| match reference.target() {
          TargetRef::Symbolic(name) => Some(text(name.shorten())),
          TargetRef::Object(_) => None,
        })
        .map(|short| {
          short
            .strip_prefix("origin/")
            .map_or_else(|| short.clone(), str::to_string)
        }),
    )
  }

  fn branch_exists(&self, branch: &str) -> Result<bool, GitFailure> {
    Ok(self.repo.try_find_reference(branch)?.is_some())
  }

  /// Where the current branch left `branch`, which is the diff base that shows
  /// this branch's work and nothing the base branch has landed since.
  pub fn merge_base(&self, branch: &str) -> Result<ObjectId, ReviewError> {
    self
      .merge_base_with_head(branch)
      .map_err(|source| ReviewError::MergeBase {
        branch: branch.to_string(),
        source,
      })
  }

  fn merge_base_with_head(&self, branch: &str) -> Result<ObjectId, GitFailure> {
    let branch = self.repo.rev_parse_single(BStr::new(branch))?.detach();
    let head = self.repo.head_id()?.detach();
    Ok(self.repo.merge_base(branch, head)?.detach())
  }

  /// The content of `path` as committed at `base`, or `None` when that commit
  /// has no such file.  Absence is a lookup that finds nothing, never a
  /// message to interpret.
  pub fn file_at(
    &self,
    base: &ObjectId,
    path: &str,
  ) -> Result<Option<String>, ReviewError> {
    self
      .committed(base, path)
      .map_err(|source| ReviewError::ConventionShow {
        path: PathBuf::from(path),
        source,
      })
  }

  fn committed(
    &self,
    base: &ObjectId,
    path: &str,
  ) -> Result<Option<String>, GitFailure> {
    self
      .tree_at(base)?
      .lookup_entry_by_path(path)?
      .map(|entry| {
        entry
          .object()
          .map(|object| String::from_utf8_lossy(&object.data).into_owned())
          .map_err(GitFailure::from)
      })
      .transpose()
  }

  /// The tree `base` names, whether it is a commit or a tree itself.
  pub(crate) fn tree_at(
    &self,
    base: &ObjectId,
  ) -> Result<gix::Tree<'_>, GitFailure> {
    Ok(self.repo.find_object(*base)?.peel_to_tree()?)
  }
}

/// The change set as the status walk builds it, before it is sorted.
#[derive(Default)]
struct Accumulated {
  paths: BTreeSet<String>,
  origins: BTreeMap<String, String>,
}

impl Accumulated {
  fn absorb(mut self, item: &Item) -> Self {
    match item {
      Item::DirectoryContents { entry, .. } if !is_untracked_file(entry) => {}
      Item::Rewrite {
        source,
        dirwalk_entry,
        ..
      } => {
        let destination = text(dirwalk_entry.rela_path.as_bstr());
        self
          .origins
          .insert(destination.clone(), text(source.rela_path()));
        self.paths.insert(destination);
      }
      _ => {
        self.paths.insert(text(item.rela_path()));
      }
    }
    self
  }

  fn into_change_set(self) -> ChangeSet {
    ChangeSet {
      paths: self.paths.into_iter().collect(),
      origins: self.origins,
    }
  }
}

/// Whether a directory-walk entry is a file git does not track, as opposed to
/// a directory, an ignored path, or something git could not track at all.
fn is_untracked_file(entry: &gix::dir::Entry) -> bool {
  entry.status == Status::Untracked
    && matches!(entry.disk_kind, Some(Kind::File | Kind::Symlink))
}

/// A path as git holds it, in text; a path that is not UTF-8 is shown as
/// closely as it can be rather than dropped.
fn text(path: &BStr) -> String {
  path.to_str_lossy().into_owned()
}

/// Drop the review record from a change set, complaining once when it is
/// there.
fn without_record(set: ChangeSet, listing: &str) -> ChangeSet {
  ChangeSet {
    paths: without_record_paths(set.paths, listing),
    origins: set.origins,
  }
}

/// Drop the review record from a listing, complaining once when it is there.
/// Its presence means git is not ignoring it, which is a real misconfiguration
/// rather than something to pass over in silence.
fn without_record_paths(paths: Vec<String>, listing: &str) -> Vec<String> {
  let (record, rest): (Vec<String>, Vec<String>) = paths
    .into_iter()
    .partition(|path| path.starts_with(RECORD_PREFIX));
  if !record.is_empty() {
    warn!(
      listing,
      "the review record is not ignored by git; add `{RECORD_PREFIX}*` to \
       .gitignore so it stays out of the tree under review"
    );
  }
  rest
}

fn non_empty_lines(text: &str) -> impl Iterator<Item = String> + '_ {
  text
    .lines()
    .filter(|line| !line.is_empty())
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_record_is_never_in_a_listing() {
    assert_eq!(
      without_record_paths(
        vec![
          "review.json".to_string(),
          "review.json.tmp9Z".to_string(),
          "src/main.rs".to_string(),
        ],
        "test",
      ),
      vec!["src/main.rs".to_string()],
    );
  }
}
