//! Semantic, per-operation errors for the dependency-bump engine.

use std::path::PathBuf;
use std::process::ExitStatus;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DependencyBumpError {
  #[error("could not read the workspace manifest {path}: {source}")]
  WorkspaceManifestReadError {
    path: PathBuf,
    source: std::io::Error,
  },
  #[error("could not parse the hold table in {path}: {source}")]
  HoldTableParseError {
    path: PathBuf,
    source: toml::de::Error,
  },
  #[error("could not read the lockfile {path}: {source}")]
  LockfileReadError {
    path: PathBuf,
    source: std::io::Error,
  },
  #[error("could not parse the lockfile {path}: {source}")]
  LockfileParseError {
    path: PathBuf,
    source: toml::de::Error,
  },
  #[error("could not run git to verify the lockfile is clean: {source}")]
  LockfileCleanCheckError { source: std::io::Error },
  #[error("git could not report the lockfile state (exited with {status})")]
  LockfileCleanCheckFailedError { status: ExitStatus },
  #[error(
    "the lockfile already has uncommitted changes; commit or restore them \
     first so the bump report reflects only this run"
  )]
  LockfileDirtyError,
  #[error("could not run cargo update: {source}")]
  CargoUpdateSpawnError { source: std::io::Error },
  #[error("cargo update exited with {status}")]
  CargoUpdateFailedError { status: ExitStatus },
  #[error(
    "could not run cargo update to re-pin held package {package}: {source}"
  )]
  HoldRepinSpawnError {
    package: String,
    source: std::io::Error,
  },
  #[error("re-pinning held package {package} exited with {status}")]
  HoldRepinFailedError { package: String, status: ExitStatus },
  #[error("could not run changelog-roller for the {package} bump: {source}")]
  ChangelogInsertSpawnError {
    package: String,
    source: std::io::Error,
  },
  #[error("changelog-roller for the {package} bump exited with {status}")]
  ChangelogInsertFailedError { package: String, status: ExitStatus },
  #[error("could not run org-fmt on {path}: {source}")]
  OrgFmtSpawnError {
    path: PathBuf,
    source: std::io::Error,
  },
  #[error("org-fmt on {path} exited with {status}")]
  OrgFmtFailedError { path: PathBuf, status: ExitStatus },
  #[error("could not write the bump report to {path}: {source}")]
  ReportWriteError {
    path: PathBuf,
    source: std::io::Error,
  },
}
