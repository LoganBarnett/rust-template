//! Resolving the three foundation pins a spawn carries.
//!
//! A spawn consumes rust-template through two independently-pinned edges:
//! - the `rust-template-foundation` git dependency, whose resolved revision is
//!   recorded in the spawn's `Cargo.lock`, and
//! - the `foundation` flake input, whose revision is recorded in `flake.lock`.
//!
//! The template's own current `HEAD` is the third reference.  `pins-agree`
//! checks the two spawn edges against each other; `pins-current` checks them
//! against the template `HEAD`.  All three are read locally — no network.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// The template's current `HEAD` commit, via `git rev-parse` run in
/// `template_dir`.  Returns `Err` with a human reason on any git failure.
pub fn template_head(template_dir: &Path) -> Result<String, String> {
  // `current_dir` is used instead of git's `-C` so no short-only flag is
  // needed at all.
  let output = Command::new("git")
    .current_dir(template_dir)
    .args(["rev-parse", "HEAD"])
    .output()
    .map_err(|error| {
      format!("could not run git in {}: {error}", template_dir.display())
    })?;
  if !output.status.success() {
    return Err(format!(
      "git rev-parse HEAD failed in {}: {}",
      template_dir.display(),
      String::from_utf8_lossy(&output.stderr).trim()
    ));
  }
  Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[derive(Debug, Deserialize)]
struct CargoLock {
  #[serde(default)]
  package: Vec<LockPackage>,
}

#[derive(Debug, Deserialize)]
struct LockPackage {
  name: String,
  #[serde(default)]
  source: Option<String>,
}

/// The foundation revision pinned in a spawn's `Cargo.lock` text, or `None`
/// when foundation is absent or not consumed as a git dependency (e.g. a path
/// override during local development).
pub fn cargo_foundation_rev(lock_text: &str) -> Result<Option<String>, String> {
  let lock: CargoLock = toml::from_str(lock_text)
    .map_err(|error| format!("could not parse Cargo.lock: {error}"))?;
  Ok(
    lock
      .package
      .into_iter()
      .find(|package| package.name == "rust-template-foundation")
      .and_then(|package| package.source)
      .and_then(|source| rev_from_git_source(&source)),
  )
}

/// A cargo git source looks like
/// `git+https://github.com/.../rust-template.git?...#<rev>`; the revision is
/// the fragment after `#`.
fn rev_from_git_source(source: &str) -> Option<String> {
  source.rsplit_once('#').map(|(_, rev)| rev.to_string())
}

#[derive(Debug, Deserialize)]
struct FlakeLock {
  #[serde(default)]
  nodes: BTreeMap<String, FlakeNode>,
}

#[derive(Debug, Deserialize)]
struct FlakeNode {
  #[serde(default)]
  locked: Option<FlakeLocked>,
}

#[derive(Debug, Deserialize)]
struct FlakeLocked {
  #[serde(default)]
  rev: Option<String>,
}

/// The foundation revision pinned by the `foundation` input in a spawn's
/// `flake.lock` text, or `None` when the input is absent.
pub fn flake_foundation_rev(lock_text: &str) -> Result<Option<String>, String> {
  let lock: FlakeLock = serde_json::from_str(lock_text)
    .map_err(|error| format!("could not parse flake.lock: {error}"))?;
  Ok(
    lock
      .nodes
      .get("foundation")
      .and_then(|node| node.locked.as_ref())
      .and_then(|locked| locked.rev.clone()),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn extracts_cargo_git_rev() {
    let lock = r#"
[[package]]
name = "rust-template-foundation"
version = "0.10.0"
source = "git+https://github.com/LoganBarnett/rust-template.git#abc123def"
"#;
    assert_eq!(
      cargo_foundation_rev(lock).unwrap(),
      Some("abc123def".to_string())
    );
  }

  #[test]
  fn cargo_path_dependency_has_no_rev() {
    let lock = r#"
[[package]]
name = "rust-template-foundation"
version = "0.10.0"
"#;
    assert_eq!(cargo_foundation_rev(lock).unwrap(), None);
  }

  #[test]
  fn extracts_flake_rev() {
    let lock = r#"
{
  "nodes": {
    "foundation": { "locked": { "rev": "abc123def", "type": "github" } },
    "nixpkgs": { "locked": { "rev": "999" } }
  },
  "root": "root",
  "version": 7
}
"#;
    assert_eq!(
      flake_foundation_rev(lock).unwrap(),
      Some("abc123def".to_string())
    );
  }

  #[test]
  fn missing_flake_foundation_input_is_none() {
    let lock = r#"{ "nodes": { "nixpkgs": {} }, "version": 7 }"#;
    assert_eq!(flake_foundation_rev(lock).unwrap(), None);
  }
}
