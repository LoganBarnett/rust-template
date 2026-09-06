use std::path::PathBuf;
use thiserror::Error;

/// How a repository read failed.  Each operation-named `ReviewError` carries
/// one of these so the operation names itself in the error rather than
/// splitting into a separate variant per failure mode.  The library's error
/// types are boxed: several are large, and an error that widens every `Result`
/// in the crate is what clippy's `result_large_err` guards against.
#[derive(Debug, Error)]
pub enum GitFailure {
  #[error("could not open the repository: {0}")]
  Open(#[source] Box<gix::discover::Error>),
  #[error("the repository has no working tree")]
  Bare,
  #[error("could not prepare the status walk: {0}")]
  Status(#[source] Box<gix::status::Error>),
  #[error("could not start the status walk: {0}")]
  StatusStart(#[source] Box<gix::status::into_iter::Error>),
  #[error("the status walk failed: {0}")]
  StatusWalk(#[source] Box<gix::status::index_worktree::Error>),
  #[error("could not build an index from the base tree: {0}")]
  IndexFromTree(#[source] Box<gix::repository::index_from_tree::Error>),
  #[error("could not resolve the revision: {0}")]
  Revision(#[source] Box<gix::revision::spec::parse::single::Error>),
  #[error("could not read `HEAD`: {0}")]
  Head(#[source] Box<gix::reference::find::existing::Error>),
  #[error("could not peel `HEAD` to a commit: {0}")]
  HeadPeel(#[source] Box<gix::head::peel::Error>),
  #[error("could not resolve the commit `HEAD` names: {0}")]
  HeadId(#[source] Box<gix::reference::head_id::Error>),
  #[error("could not look up the reference: {0}")]
  Reference(#[source] Box<gix::reference::find::Error>),
  #[error("no merge base: {0}")]
  MergeBase(#[source] Box<gix::repository::merge_base::Error>),
  #[error("could not read the object: {0}")]
  Object(#[source] Box<gix::object::find::existing::Error>),
  #[error("could not peel the object to a tree: {0}")]
  PeelToTree(#[source] Box<gix::object::peel::to_kind::Error>),
  #[error("could not prepare the diff pipeline: {0}")]
  DiffCache(#[source] Box<gix::repository::diff_resource_cache::Error>),
  #[error("could not load a side of the diff: {0}")]
  DiffResource(#[source] Box<gix::diff::blob::platform::set_resource::Error>),
  #[error("could not prepare the diff: {0}")]
  DiffPrepare(#[source] Box<gix::diff::blob::platform::prepare_diff::Error>),
  #[error("could not read the diff algorithm from the git config: {0}")]
  DiffAlgorithm(#[source] Box<gix::config::diff::algorithm::Error>),
  #[error("could not render the diff: {0}")]
  DiffRender(#[source] Box<std::io::Error>),
  #[error("could not hash the content: {0}")]
  Hash(#[source] Box<gix::hash::hasher::Error>),
}

/// `?` conversions into the boxed variants; `#[from]` would take the box
/// itself as the source.
macro_rules! boxed_from {
  ($($variant:ident: $source:ty),* $(,)?) => {$(
    impl From<$source> for GitFailure {
      fn from(source: $source) -> Self {
        Self::$variant(Box::new(source))
      }
    }
  )*};
}

boxed_from! {
  Open: gix::discover::Error,
  Status: gix::status::Error,
  StatusStart: gix::status::into_iter::Error,
  StatusWalk: gix::status::index_worktree::Error,
  IndexFromTree: gix::repository::index_from_tree::Error,
  Revision: gix::revision::spec::parse::single::Error,
  Head: gix::reference::find::existing::Error,
  HeadPeel: gix::head::peel::Error,
  HeadId: gix::reference::head_id::Error,
  Reference: gix::reference::find::Error,
  MergeBase: gix::repository::merge_base::Error,
  Object: gix::object::find::existing::Error,
  PeelToTree: gix::object::peel::to_kind::Error,
  DiffCache: gix::repository::diff_resource_cache::Error,
  DiffResource: gix::diff::blob::platform::set_resource::Error,
  DiffPrepare: gix::diff::blob::platform::prepare_diff::Error,
  DiffAlgorithm: gix::config::diff::algorithm::Error,
  DiffRender: std::io::Error,
  Hash: gix::hash::hasher::Error,
}

/// Every way a review can fail.  A failure is never a pass: the caller reports
/// it and exits non-zero rather than treating an unreachable reviewer as a
/// clean tree.
#[derive(Debug, Error)]
pub enum ReviewError {
  #[error(
    "could not list the changes between the diff base and the working tree: \
     {0}"
  )]
  WorkingTreeList(#[source] GitFailure),
  #[error("could not list the untracked files for the review packet: {0}")]
  UntrackedList(#[source] GitFailure),
  #[error(
    "could not read the working-tree file-list fixture at {path:?}: {source}"
  )]
  WorkingTreeFixtureRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "could not open the repository enclosing the current directory: {0}"
  )]
  RepositoryOpen(#[source] GitFailure),
  #[error("could not resolve the commit under review from `HEAD`: {0}")]
  HeadResolve(#[source] GitFailure),
  #[error("could not resolve the default branch: {0}")]
  DefaultBranchResolve(#[source] GitFailure),
  #[error(
    "could not determine the default branch to diff against: neither \
     `origin/HEAD` nor a local `main` or `master` resolves.  Pass \
     `--diff head` to review the working tree instead."
  )]
  DefaultBranchUnresolved,
  #[error("could not find where the current branch left {branch:?}: {source}")]
  MergeBase {
    branch: String,
    #[source]
    source: GitFailure,
  },
  #[error("could not render the review packet's diff: {0}")]
  PacketDiff(#[source] GitFailure),
  #[error(
    "could not read the convention document {path:?} as committed at the \
     diff base: {source}"
  )]
  ConventionShow {
    path: PathBuf,
    #[source]
    source: GitFailure,
  },
  #[error("could not read the convention document {path:?}: {source}")]
  ConventionRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not read the global instructions at {path:?}: {source}")]
  GlobalInstructionsRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "could not read the untracked file {path:?} for the review packet: \
     {source}"
  )]
  UntrackedFileRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not hash the changed files' content: {0}")]
  Fingerprint(#[source] GitFailure),
  #[error(
    "could not read {path:?} to hash its content for the review record: \
     {source}"
  )]
  FingerprintRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not read the review record at {path:?}: {source}")]
  RecordRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "the review record at {path:?} is not valid JSON: {source}.  Delete it \
     to start a fresh review."
  )]
  RecordParse {
    path: PathBuf,
    #[source]
    source: serde_json::Error,
  },
  #[error("could not serialize the review record: {0}")]
  RecordSerialize(#[source] serde_json::Error),
  #[error(
    "could not create the review record's temporary file in {dir:?}: {source}"
  )]
  RecordTempCreate {
    dir: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not write the review record to {path:?}: {source}")]
  RecordPersist {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not remove the stale review record at {path:?}: {source}")]
  RecordRemove {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "the `claude` command is not on PATH, so the review cannot run.  Install \
     Claude Code, or set `review_reviewer_cmd` to a stand-in."
  )]
  ReviewerNotOnPath,
  #[error("could not write the reviewer's prompt file in {path:?}: {source}")]
  ReviewerPromptWrite {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not run the reviewer ({command}): {source}")]
  ReviewerInvocation {
    command: String,
    #[source]
    source: std::io::Error,
  },
  #[error("the reviewer exited {status}: {stderr}")]
  ReviewerFailed { status: String, stderr: String },
  #[error("the reviewer did not finish within {secs} seconds and was stopped")]
  ReviewerTimedOut { secs: u64 },
  #[error(
    "the reviewer exited without reading the packet, so it judged nothing"
  )]
  ReviewerIgnoredPacket,
  #[error("the reviewer reported an error rather than a verdict: {message}")]
  ReviewerReportedError { message: String },
  #[error(
    "the reviewer's output is not a verdict: {source}.  The first {} \
     characters were: {excerpt}",
    excerpt.len()
  )]
  VerdictUnparseable {
    excerpt: String,
    #[source]
    source: serde_json::Error,
  },
  #[error(
    "the reviewer's output carries neither a structured verdict nor one as \
     text.  The first {} characters were: {excerpt}",
    excerpt.len()
  )]
  VerdictMissing { excerpt: String },
  #[error("the thread reading the reviewer's {stream} panicked: {detail}")]
  ReviewerOutputThreadPanicked {
    stream: &'static str,
    detail: String,
  },
}

/// A panicked thread's payload as text.  A panic carries either a `&str` or a
/// `String` in practice; anything else has no message to recover.
pub fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
  payload
    .downcast_ref::<&str>()
    .map(|message| (*message).to_string())
    .or_else(|| payload.downcast_ref::<String>().cloned())
    .unwrap_or_else(|| "no panic message".to_string())
}
