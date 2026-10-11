//! A spawn's provenance file — `rust-template.json` at its project root.
//!
//! Beyond the `template_sync_hashes` list (used by the compliance *process*,
//! not this checker), the file may carry a `compliance-ignores` key listing the
//! checks a project has deliberately opted out of.  An ignored check is
//! reported distinctly and does not count as a failure.  The `workspaces` map
//! records each workspace member's release-binary flag and archetype role.
//! The release-binary workflow reads the flag.  The checker reads the role to
//! follow a role crate emitted under a custom name.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;

/// Parsed `rust-template.json`.  A missing or unparseable file degrades to the
/// default, which records no ignores.  The `provenance-schema-valid` check
/// reports an unparseable file, so the loader never aborts the run.
#[derive(Debug, Default, Deserialize)]
pub struct Provenance {
  #[serde(default)]
  pub template_sync_hashes: Vec<String>,
  #[serde(default, rename = "compliance-ignores")]
  pub compliance_ignores: Vec<Ignore>,
  #[serde(default)]
  pub workspaces: BTreeMap<String, WorkspaceConfig>,
}

/// Per-member settings from the `workspaces` map.  Every workspace member the
/// template adds is recorded here.  `release-binary` is `true` for the binary
/// archetypes, cli and server, and `false` for libs.  `type` names the
/// member's archetype role whatever its directory is called.
#[derive(Debug, Deserialize)]
pub struct WorkspaceConfig {
  #[serde(default, rename = "release-binary")]
  pub release_binary: bool,
  #[serde(rename = "type")]
  pub crate_type: CrateRole,
}

/// The archetype role a workspace member plays.  The template names its
/// archetypes `cli`, `server`, and `lib`.  A spawn may put a role crate under
/// any directory name and declare the role here.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CrateRole {
  Cli,
  Server,
  Lib,
}

impl CrateRole {
  pub const ALL: [CrateRole; 3] =
    [CrateRole::Cli, CrateRole::Server, CrateRole::Lib];

  pub fn as_str(self) -> &'static str {
    match self {
      CrateRole::Cli => "cli",
      CrateRole::Server => "server",
      CrateRole::Lib => "lib",
    }
  }

  pub fn parse(value: &str) -> Option<CrateRole> {
    Self::ALL.into_iter().find(|role| role.as_str() == value)
  }
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

  /// The workspace member declaring `role`, if any.
  fn role_member(&self, role: CrateRole) -> Option<&str> {
    self
      .workspaces
      .iter()
      .find(|(_, config)| config.crate_type == role)
      .map(|(name, _)| name.as_str())
  }

  /// Whether a workspace member declares `role`.  A declared role counts even
  /// when its directory is missing.  The role's checks then run and fail
  /// instead of silently skipping.
  pub fn has_role(&self, role: CrateRole) -> bool {
    self.role_member(role).is_some()
  }

  /// Rewrite a path under `crates/<role>` to the directory of the member that
  /// declares that role.  Every other path comes back unchanged.  That covers
  /// `crates/*` globs, paths under other crate directories, and role paths no
  /// member declares.
  pub fn resolve_role_path(&self, path: &str) -> String {
    CrateRole::ALL
      .into_iter()
      .find_map(|role| {
        path
          .strip_prefix(&format!("crates/{}", role.as_str()))
          .filter(|rest| rest.is_empty() || rest.starts_with('/'))
          .and_then(|rest| {
            self
              .role_member(role)
              .map(|member| format!("crates/{member}{rest}"))
          })
      })
      .unwrap_or_else(|| path.to_string())
  }
}

/// Load a spawn's provenance.  A missing file is normal for older spawns.  An
/// unreadable file, or one that breaks the schema, is logged and degraded to
/// the default so the spawn's other checks still run.  A `workspaces` entry
/// without its `type` breaks the schema.  The `provenance-schema-valid` check
/// reports the degradation as a failure.
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

#[cfg(test)]
mod tests {
  use super::*;

  fn provenance(json: &str) -> Provenance {
    serde_json::from_str(json).unwrap()
  }

  #[test]
  fn duplicate_declarations_pick_the_first_in_key_order() {
    let p = provenance(
      r#"{ "workspaces": {
        "beta": { "release-binary": true, "type": "cli" },
        "alpha": { "release-binary": true, "type": "cli" }
      } }"#,
    );
    assert_eq!(p.role_member(CrateRole::Cli), Some("alpha"));
  }

  #[test]
  fn a_member_name_carries_no_role() {
    // An entry named `cli` declaring itself a server plays the server role
    // and leaves the cli role undeclared.
    let p = provenance(
      r#"{ "workspaces": {
        "cli": { "release-binary": true, "type": "server" }
      } }"#,
    );
    assert_eq!(p.role_member(CrateRole::Server), Some("cli"));
    assert_eq!(p.role_member(CrateRole::Cli), None);
  }

  #[test]
  fn an_undeclared_role_leaves_its_paths_unresolved() {
    let p = provenance(r#"{ "workspaces": {} }"#);
    assert_eq!(p.role_member(CrateRole::Server), None);
    assert_eq!(
      p.resolve_role_path("crates/server/src/main.rs"),
      "crates/server/src/main.rs"
    );
  }

  #[test]
  fn resolves_role_prefixed_paths_only() {
    let p = provenance(
      r#"{ "workspaces": {
        "provider": { "release-binary": true, "type": "cli" }
      } }"#,
    );
    assert_eq!(
      p.resolve_role_path("crates/cli/src/config.rs"),
      "crates/provider/src/config.rs"
    );
    assert_eq!(p.resolve_role_path("crates/cli"), "crates/provider");
    assert_eq!(
      p.resolve_role_path("crates/other/lib.rs"),
      "crates/other/lib.rs"
    );
    assert_eq!(
      p.resolve_role_path("crates/*/Cargo.toml"),
      "crates/*/Cargo.toml"
    );
    assert_eq!(p.resolve_role_path("crates/clip/x.rs"), "crates/clip/x.rs");
    assert_eq!(p.resolve_role_path("flake.nix"), "flake.nix");
  }

  #[test]
  fn a_role_is_present_only_when_declared() {
    let p = provenance(
      r#"{ "workspaces": {
        "provider": { "release-binary": true, "type": "server" }
      } }"#,
    );
    assert!(p.has_role(CrateRole::Server));
    assert!(!p.has_role(CrateRole::Cli));
  }

  #[test]
  fn a_workspaces_entry_without_a_type_fails_deserialization() {
    let result = serde_json::from_str::<Provenance>(
      r#"{ "workspaces": { "cli": { "release-binary": true } } }"#,
    );
    assert!(result.is_err());
  }

  #[test]
  fn load_degrades_a_legacy_file_to_the_default() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
      tmp.path().join("rust-template.json"),
      r#"{ "workspaces": { "cli": { "release-binary": true } } }"#,
    )
    .unwrap();
    let p = load(tmp.path());
    assert!(p.workspaces.is_empty());
  }
}
