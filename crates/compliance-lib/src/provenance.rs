//! A spawn's provenance file — `rust-template.json` at its project root.
//!
//! Beyond the `template_sync_hashes` list (used by the compliance *process*,
//! not this checker), the file may carry a `compliance-ignores` key listing the
//! checks a project has deliberately opted out of.  An ignored check is
//! reported distinctly and does not count as a failure.

use serde::Deserialize;
use std::io::ErrorKind;
use std::path::Path;

/// Parsed `rust-template.json`.  Missing or unparseable files degrade to the
/// default (no recorded ignores); the `json-valid` check is what flags an
/// unparseable provenance file, so the loader does not abort the run.
#[derive(Debug, Default, Deserialize)]
pub struct Provenance {
  #[serde(default)]
  pub template_sync_hashes: Vec<String>,
  #[serde(default, rename = "compliance-ignores")]
  pub compliance_ignores: Vec<Ignore>,
}

/// A single `compliance-ignores` entry: either a bare check id, or an object
/// pairing the id with a human-readable reason.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Ignore {
  Id(String),
  Detailed {
    id: String,
    #[serde(default)]
    reason: Option<String>,
  },
}

impl Ignore {
  fn id(&self) -> &str {
    match self {
      Ignore::Id(id) => id,
      Ignore::Detailed { id, .. } => id,
    }
  }

  fn reason(&self) -> Option<&str> {
    match self {
      Ignore::Id(_) => None,
      Ignore::Detailed { reason, .. } => reason.as_deref(),
    }
  }
}

impl Provenance {
  /// If `id` is ignored, return its reason (the inner `Option` is the
  /// optional explanation); return `None` when the check is not ignored.
  pub fn ignored_reason(&self, id: &str) -> Option<Option<String>> {
    self
      .compliance_ignores
      .iter()
      .find(|ignore| ignore.id() == id)
      .map(|ignore| ignore.reason().map(str::to_string))
  }
}

/// Load a spawn's provenance.  A missing file is normal (older spawns); an
/// unreadable or unparseable file is logged and degraded to the default so the
/// rest of the spawn's checks still run.
pub fn load(dir: &Path) -> Provenance {
  let path = dir.join("rust-template.json");
  let text = match std::fs::read_to_string(&path) {
    Ok(text) => text,
    Err(error) if error.kind() == ErrorKind::NotFound => {
      return Provenance::default()
    }
    Err(error) => {
      tracing::warn!("could not read {}: {error}", path.display());
      return Provenance::default();
    }
  };
  match serde_json::from_str(&text) {
    Ok(provenance) => provenance,
    Err(error) => {
      tracing::warn!("ignoring unparseable {}: {error}", path.display());
      Provenance::default()
    }
  }
}
