//! The spawn registry — `config.json` at the template repo root.
//!
//! It records every project generated from the template, where it lives on
//! disk, the crate roles it was spawned with, and whether it has been
//! archived.  A compliance run iterates this registry the same way the legacy
//! `compliance-check.sh` did.

use crate::error::ComplianceError;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The parsed `config.json`.
#[derive(Debug, Deserialize)]
pub struct Registry {
  #[serde(rename = "templateSpawns")]
  pub template_spawns: BTreeMap<String, Spawn>,
}

/// One registered spawn.
#[derive(Debug, Deserialize)]
pub struct Spawn {
  pub dir: String,
  #[serde(default)]
  pub archived: bool,
  #[serde(default)]
  pub args: SpawnArgs,
}

/// The generation arguments recorded for a spawn.  `crates` is a
/// comma-separated role list (e.g. `"cli,server"`) used to decide which
/// role-conditional checks apply.
#[derive(Debug, Default, Deserialize)]
pub struct SpawnArgs {
  #[serde(default)]
  pub crates: String,
  #[serde(default)]
  pub description: String,
  #[serde(default)]
  pub public: bool,
}

/// Load and parse the spawn registry at `path`.
pub fn load(path: &Path) -> Result<Registry, ComplianceError> {
  let text = std::fs::read_to_string(path).map_err(|source| {
    ComplianceError::RegistryRead {
      path: path.to_path_buf(),
      source,
    }
  })?;
  serde_json::from_str(&text).map_err(|source| ComplianceError::RegistryParse {
    path: path.to_path_buf(),
    source,
  })
}
