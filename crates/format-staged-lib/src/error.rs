use std::path::PathBuf;
use std::process::ExitStatus;
use thiserror::Error;

/// How running a file through git's clean filters failed.  The engine cleans
/// two kinds of file, so each operation-named `FormatStagedError` carries one
/// of these rather than splitting into a variant per failure mode.
#[derive(Debug, Error)]
pub enum CleanFailure {
  #[error("could not open the file: {0}")]
  Open(#[source] std::io::Error),
  #[error("could not apply git's filters: {0}")]
  Filter(#[source] gix::Error),
  #[error("could not read the filtered content: {0}")]
  Read(#[source] std::io::Error),
}

/// Every way a run can fail.  A failure never leaves the index half
/// rewritten: the index is written once, after every blob exists.
#[derive(Debug, Error)]
pub enum FormatStagedError {
  #[error(
    "could not open the repository enclosing the current directory: {0}"
  )]
  RepositoryOpen(#[source] gix::Error),
  #[error(
    "the repository has no working tree, so it has no staged content to \
     format"
  )]
  RepositoryBare,
  #[error(
    "`core.splitIndex` is set, and a split index cannot be rewritten without \
     risking its entries.  Unset `core.splitIndex` and run `git update-index \
     --no-split-index`."
  )]
  IndexSplit,
  #[error("could not read the index at {path:?}: {source}")]
  IndexRead {
    path: PathBuf,
    #[source]
    source: gix::Error,
  },
  #[error(
    "the index at {path:?} is sparse, and a sparse index cannot be exported \
     for formatting.  Turn it off with `git sparse-checkout init \
     --no-sparse-index`."
  )]
  IndexSparse { path: PathBuf },
  #[error("could not resolve the tree `HEAD` names: {0}")]
  HeadTreeResolve(#[source] gix::Error),
  #[error(
    "could not read the `HEAD` tree to tell which index entries are staged: \
     {0}"
  )]
  HeadIndexBuild(#[source] gix::Error),
  #[error(
    "the staged path {path:?} cannot be a file path on this platform: {source}"
  )]
  StagedPathConvert {
    path: String,
    #[source]
    source: gix::Error,
  },
  #[error("could not create a directory to export the index into: {0}")]
  ExportDirCreate(#[source] std::io::Error),
  #[error("could not resolve the export directory {dir:?}: {source}")]
  ExportDirResolve {
    dir: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error("could not read the checkout settings for the index export: {0}")]
  ExportOptions(#[source] gix::Error),
  #[error("could not open the object database for the index export: {0}")]
  ExportObjects(#[source] std::io::Error),
  #[error("could not export the index to {dir:?}: {source}")]
  Export {
    dir: PathBuf,
    #[source]
    source: gix::Error,
  },
  #[error(
    "exporting the index put two paths in the same place: {}.  The \
     filesystem cannot tell them apart, so their staged content cannot be \
     formatted safely.",
    paths.join(", ")
  )]
  ExportCollision { paths: Vec<String> },
  #[error(
    "could not run treefmt to format the staged content: {source}.  The \
     packaged format-staged carries treefmt.  A bare binary needs it on PATH."
  )]
  FormatterInvocation {
    #[source]
    source: std::io::Error,
  },
  #[error("treefmt could not format the staged content ({status})")]
  FormatterFailed { status: ExitStatus },
  #[error("could not prepare git's filters for the formatted content: {0}")]
  FilterPipeline(#[source] gix::Error),
  #[error("could not take the formatted content of {path:?}: {source}")]
  FormattedContentClean {
    path: PathBuf,
    #[source]
    source: CleanFailure,
  },
  #[error("could not store the formatted content of {path:?}: {source}")]
  FormattedBlobWrite {
    path: PathBuf,
    #[source]
    source: gix::Error,
  },
  #[error("could not inspect the working file {path:?}: {source}")]
  WorkingFileStat {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
  #[error(
    "could not compare the working file {path:?} with its staged content: \
     {source}"
  )]
  WorkingFileClean {
    path: PathBuf,
    #[source]
    source: CleanFailure,
  },
  #[error("could not hash the working file {path:?}: {source}")]
  WorkingFileHash {
    path: PathBuf,
    #[source]
    source: gix::Error,
  },
  #[error("the staged path {path:?} is no longer in the index")]
  IndexEntryMissing { path: String },
  #[error(
    "could not write the formatted content to the index at {path:?}: {source}"
  )]
  IndexWrite {
    path: PathBuf,
    #[source]
    source: gix::Error,
  },
  #[error(
    "could not update the working file {path:?} with its formatted content: \
     {source}.  The index already holds the formatted content."
  )]
  WorkingFileSync {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },
}
