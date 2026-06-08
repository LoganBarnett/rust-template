//! Top-level errors for loading the inputs a compliance run needs.
//!
//! These are the failures that abort the whole run because nothing can proceed
//! without them: the spawn registry and the check manifest.  Per-check and
//! per-spawn failures are *not* errors — they are reported as
//! [`crate::check::CheckOutcome`] values so one bad spawn cannot mask the rest.

use std::path::PathBuf;
use thiserror::Error;

/// A failure that prevents a compliance run from starting.
#[derive(Debug, Error)]
pub enum ComplianceError {
  #[error("could not read spawn registry {path}: {source}")]
  RegistryRead {
    path: PathBuf,
    source: std::io::Error,
  },

  #[error("could not parse spawn registry {path}: {source}")]
  RegistryParse {
    path: PathBuf,
    source: serde_json::Error,
  },

  #[error("could not read compliance manifest {path}: {source}")]
  ManifestRead {
    path: PathBuf,
    source: std::io::Error,
  },

  #[error("could not parse compliance manifest {path}: {source}")]
  ManifestParse {
    path: PathBuf,
    source: toml::de::Error,
  },

  /// A manifest entry parsed as TOML but is not a valid check definition
  /// (unknown kind, or a required parameter for its kind is missing).
  #[error("invalid check '{id}': {message}")]
  ManifestInvalid { id: String, message: String },
}
