//! The hold table: packages the bump must not advance, declared under
//! `[workspace.metadata.dependency-bump]` in the workspace manifest.
//!
//! A hold carries a mandatory reason so the "why" travels with the "what":
//! every run reports the held set, and a hold with no justification cannot
//! exist.  Richer rules (bump A only alongside B; hold until an issue's
//! fix ships in a release tag) are planned on this same table — see
//! tasks.org.

use crate::error::DependencyBumpError;
use serde::Deserialize;
use std::path::Path;

/// One held package: excluded from the bump until a human lifts the hold.
#[derive(Debug, Clone, Deserialize)]
pub struct Hold {
  /// The lockfile package name held at its current version.
  pub package: String,
  /// Why the hold exists; surfaced in every run's output.
  pub reason: String,
}

/// The `[workspace.metadata.dependency-bump]` table shape.
#[derive(Debug, Default, Deserialize)]
struct Policy {
  #[serde(default)]
  hold: Vec<Hold>,
}

/// Reads the hold set from the workspace manifest.  A manifest without the
/// metadata table holds nothing — that is the common case, not an error.
pub fn holds(manifest_path: &Path) -> Result<Vec<Hold>, DependencyBumpError> {
  std::fs::read_to_string(manifest_path)
    .map_err(|source| DependencyBumpError::WorkspaceManifestReadError {
      path: manifest_path.to_path_buf(),
      source,
    })
    .and_then(|manifest| {
      parse_holds(&manifest).map_err(|source| {
        DependencyBumpError::HoldTableParseError {
          path: manifest_path.to_path_buf(),
          source,
        }
      })
    })
}

fn parse_holds(manifest: &str) -> Result<Vec<Hold>, toml::de::Error> {
  toml::from_str::<toml::Value>(manifest).and_then(|value| {
    value
      .get("workspace")
      .and_then(|workspace| workspace.get("metadata"))
      .and_then(|metadata| metadata.get("dependency-bump"))
      .cloned()
      .map_or(Ok(Vec::new()), |policy| {
        policy.try_into::<Policy>().map(|parsed| parsed.hold)
      })
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_holds_with_reasons() {
    let manifest = r#"
      [workspace.metadata.dependency-bump]
      hold = [
        { package = "libc", reason = "waiting on a fix" },
        { package = "serde", reason = "coupled to serde_derive" },
      ]
    "#;
    let holds = parse_holds(manifest).unwrap();
    assert_eq!(holds.len(), 2);
    assert_eq!(holds[0].package, "libc");
    assert_eq!(holds[0].reason, "waiting on a fix");
  }

  #[test]
  fn missing_table_means_no_holds() {
    let manifest = r#"
      [workspace]
      members = []
    "#;
    assert!(parse_holds(manifest).unwrap().is_empty());
  }

  #[test]
  fn empty_hold_list_means_no_holds() {
    let manifest = r#"
      [workspace.metadata.dependency-bump]
      hold = []
    "#;
    assert!(parse_holds(manifest).unwrap().is_empty());
  }

  #[test]
  fn a_hold_without_a_reason_is_an_error() {
    let manifest = r#"
      [workspace.metadata.dependency-bump]
      hold = [{ package = "libc" }]
    "#;
    assert!(parse_holds(manifest).is_err());
  }

  #[test]
  fn malformed_manifest_is_an_error() {
    assert!(parse_holds("not toml [").is_err());
  }
}
