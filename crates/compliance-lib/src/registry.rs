//! The spawn registry — `config.json` at the template repo root.
//!
//! It records where each spawn lives on disk and whether it is archived.
//! Everything else, crate roles included, is read from the spawn's own
//! `rust-template.json`.

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
