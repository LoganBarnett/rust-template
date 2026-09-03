use std::path::PathBuf;
use thiserror::Error;

/// How a `git` invocation failed: the process could not be run to completion
/// (spawn, the pipe to it, or the wait all surface as an I/O error), or it ran
/// and exited non-zero.  Each operation-named `AppError` carries one of these
/// so the operation names itself in the error rather than splitting into a
/// separate variant per failure mode.
#[derive(Debug, Error)]
pub enum GitFailure {
  #[error("the git process could not be run: {0}")]
  Io(#[source] std::io::Error),
  #[error("git exited non-zero: {0}")]
  Exit(String),
}

/// Every way the gate can fail outright.  In hook mode a failure is not a
/// release: `main` turns it into a block that carries the message, since a
/// gate that stands aside when it cannot run is a gate the agent can end a
/// turn through.  Only writing the decision itself can still exit non-zero.
#[derive(Debug, Error)]
pub enum AppError {
  #[error("could not read the Stop hook's stdin document: {0}")]
  HookInputRead(#[source] std::io::Error),
  #[error("could not parse the Stop hook's stdin document as JSON: {0}")]
  HookInputParse(#[source] serde_json::Error),
  #[error("could not list the working tree's changes: {0}")]
  WorkingTreeList(#[source] GitFailure),
  #[error("could not list the untracked files for the review packet: {0}")]
  PacketUntrackedList(#[source] GitFailure),
  #[error(
    "could not read the working-tree file-list fixture at {path:?}: {source}"
  )]
  WorkingTreeFixtureRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "could not locate the repository for the verdict cache key with \
     `git rev-parse --show-toplevel`: {0}"
  )]
  CacheKeyToplevel(#[source] GitFailure),
  #[error(
    "could not resolve the commit under review with `git rev-parse HEAD`: {0}"
  )]
  HeadResolve(#[source] GitFailure),
  #[error(
    "could not compute the empty-tree diff base with `git hash-object`: {0}"
  )]
  EmptyTreeHash(#[source] GitFailure),
  #[error(
    "could not assemble the review packet's change set with `git diff`: {0}"
  )]
  PacketDiff(#[source] GitFailure),
  #[error(
    "could not read the convention document {path:?} as committed at HEAD \
     with `git show`: {source}"
  )]
  ConventionShow {
    path: PathBuf,
    #[source]
    source: GitFailure,
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
  #[error(
    "could not read the convention document {path:?} from the working tree \
     for the review packet: {source}"
  )]
  ConventionRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "could not read the global instructions at {path:?} for the review \
     packet: {source}"
  )]
  GlobalInstructionsRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "could not fingerprint the qualifying files with `git hash-object`: {0}"
  )]
  Fingerprint(#[source] GitFailure),
  #[error(
    "`git hash-object` returned {hashes} hashes for {paths} qualifying files"
  )]
  FingerprintHashCount { hashes: usize, paths: usize },
  #[error(
    "could not hash the repository path for the verdict cache key with \
     `git hash-object`: {0}"
  )]
  CacheKey(#[source] GitFailure),
  #[error("could not run the clippy gate command `{command}`: {source}")]
  ClippyInvocation {
    command: String,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "the clippy gate cannot run: neither cargo nor nix is on PATH, so the \
     Rust changes cannot be checked; the environment must provide a Rust \
     toolchain for the gate"
  )]
  ClippyUnavailable,
  #[error(
    "the reviewer command `claude` is not on PATH; the gate cannot review \
     without it"
  )]
  ReviewerNotOnPath,
  #[error(
    "could not write the reviewer's prompt to a temporary file at {path:?}: \
     {source}"
  )]
  ReviewerPromptWrite {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "the reviewer exited successfully without reading the review packet, so \
     it never saw what it was asked to judge; its verdict cannot be trusted"
  )]
  ReviewerIgnoredPacket,
  #[error("could not run the reviewer command `{command}`: {source}")]
  ReviewerInvocation {
    command: String,
    #[source]
    source: std::io::Error,
  },
  #[error("the reviewer exited with {status}: {stderr}")]
  ReviewerFailed { status: String, stderr: String },
  #[error(
    "the reviewer did not finish within {secs} seconds and was stopped; run \
     the turn again, or raise review_stop_reviewer_timeout_secs (kept below \
     the Stop hook timeout)"
  )]
  ReviewerTimedOut { secs: u64 },
  #[error("the thread draining the reviewer's {stream} panicked: {detail}")]
  ReviewerOutputThreadPanicked {
    stream: &'static str,
    detail: String,
  },
  #[error("the reviewer reported an error instead of a verdict: {detail}")]
  ReviewerReportedError { detail: String },
  #[error(
    "could not read a verdict from the reviewer's output ({detail}); the \
     output was: {output}"
  )]
  VerdictUnparseable { detail: String, output: String },
  #[error("could not create the verdict cache directory {path:?}: {source}")]
  CacheDirCreate {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not read the cached verdict at {path:?}: {source}")]
  CacheRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not parse the cached verdict at {path:?}: {source}")]
  CacheParse {
    path: PathBuf,
    #[source]
    source: serde_json::Error,
  },
  #[error("could not write the cached verdict at {path:?}: {source}")]
  CacheWrite {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not serialise the verdict for the cache: {0}")]
  CacheSerialize(#[source] serde_json::Error),
  #[error("could not serialise the block decision: {0}")]
  DecisionSerialize(#[source] serde_json::Error),
  #[error("could not write the block decision to stdout: {0}")]
  DecisionWrite(#[source] std::io::Error),
  #[error("could not write the review report to stdout: {0}")]
  ReportWrite(#[source] std::io::Error),
}

/// The message a panic payload carries, when it carries one.  A thread join
/// yields the payload as an opaque `Any`; the two shapes `panic!` produces are
/// a `&str` and a `String`.
pub fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
  payload
    .downcast_ref::<&str>()
    .map(|message| (*message).to_string())
    .or_else(|| payload.downcast_ref::<String>().cloned())
    .unwrap_or_else(|| "no panic message".to_string())
}
