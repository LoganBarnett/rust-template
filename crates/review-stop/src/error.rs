use std::any::Any;
use std::path::PathBuf;
use thiserror::Error;

/// Every way the gate can fail outright.  A failure exits non-zero, which
/// Claude Code reports as a hook error rather than a block.  The gate's own
/// verdicts (release, block) are not errors; they travel on stdout.
#[derive(Debug, Error)]
pub enum AppError {
  #[error("could not read the Stop hook's stdin document: {0}")]
  HookInputRead(#[source] std::io::Error),
  #[error("could not parse the Stop hook's stdin document as JSON: {0}")]
  HookInputParse(#[source] serde_json::Error),
  #[error("could not run `{command}` to list working-tree changes: {source}")]
  WorkingTreeList {
    command: String,
    #[source]
    source: std::io::Error,
  },
  #[error("`{command}` failed while listing working-tree changes: {stderr}")]
  WorkingTreeListFailed { command: String, stderr: String },
  #[error(
    "could not read the working-tree file-list fixture at {path:?}: {source}"
  )]
  WorkingTreeFixtureRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "could not run `git hash-object` to fingerprint the qualifying files: \
     {source}"
  )]
  FingerprintHash {
    #[source]
    source: std::io::Error,
  },
  #[error(
    "`git hash-object` failed while fingerprinting the qualifying files: \
     {stderr}"
  )]
  FingerprintHashFailed { stderr: String },
  #[error(
    "`git hash-object` returned {hashes} hashes for {paths} qualifying files"
  )]
  FingerprintHashCount { hashes: usize, paths: usize },
  #[error("could not run the clippy gate command `{command}`: {source}")]
  ClippyInvocation {
    command: String,
    #[source]
    source: std::io::Error,
  },
  #[error("could not read the session transcript at {path:?}: {source}")]
  TranscriptRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "could not serialise a transcript entry while collecting verdicts: {0}"
  )]
  TranscriptEntrySerialize(#[source] serde_json::Error),
  #[error("could not read the gate's state file at {path:?}: {source}")]
  StateFileRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not write the gate's state file at {path:?}: {source}")]
  StateFileWrite {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not remove the gate's state file at {path:?}: {source}")]
  StateFileRemove {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not serialise the block decision: {0}")]
  DecisionSerialize(#[source] serde_json::Error),
  #[error("could not write the block decision to stdout: {0}")]
  DecisionWrite(#[source] std::io::Error),
  #[error("the gate's {task} thread panicked: {detail}")]
  GateThreadPanicked { task: &'static str, detail: String },
}

/// The message a panic payload carries, when it carries one.  A thread join
/// yields the payload as an opaque `Any`; the two shapes `panic!` produces are
/// a `&str` and a `String`.
pub fn panic_detail(payload: &(dyn Any + Send)) -> String {
  payload
    .downcast_ref::<&str>()
    .map(|message| (*message).to_string())
    .or_else(|| payload.downcast_ref::<String>().cloned())
    .unwrap_or_else(|| "no panic message".to_string())
}
