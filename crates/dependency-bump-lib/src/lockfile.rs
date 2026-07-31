//! Lockfile snapshots and the bump diff.
//!
//! The engine reads Cargo.lock before and after `cargo update` and derives
//! what moved from the two snapshots — the lockfile is the one place the
//! set of resolved versions cannot be reworded out from under us.  Parsing
//! the TOML directly (rather than scraping a unified diff, as the retired
//! dependabot-combine tool did with awk) keeps multi-version packages and
//! formatting noise out of the signal.

use crate::error::DependencyBumpError;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Resolved versions per package name, each list sorted.
pub type Snapshot = BTreeMap<String, Vec<String>>;

#[derive(Debug, Deserialize)]
struct Lockfile {
  #[serde(default)]
  package: Vec<LockedPackage>,
}

#[derive(Debug, Deserialize)]
struct LockedPackage {
  name: String,
  version: String,
}

/// One package that moved: `from` → `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bump {
  pub name: String,
  pub from: String,
  pub to: String,
}

/// Parses the lockfile into a snapshot of resolved versions.
pub fn snapshot(lockfile_path: &Path) -> Result<Snapshot, DependencyBumpError> {
  std::fs::read_to_string(lockfile_path)
    .map_err(|source| DependencyBumpError::LockfileReadError {
      path: lockfile_path.to_path_buf(),
      source,
    })
    .and_then(|text| {
      toml::from_str::<Lockfile>(&text).map_err(|source| {
        DependencyBumpError::LockfileParseError {
          path: lockfile_path.to_path_buf(),
          source,
        }
      })
    })
    .map(|lockfile| {
      lockfile.package.into_iter().fold(
        Snapshot::new(),
        |mut snapshot, package| {
          snapshot
            .entry(package.name)
            .or_default()
            .push(package.version);
          snapshot
        },
      )
    })
    .map(|mut snapshot| {
      snapshot.values_mut().for_each(|versions| versions.sort());
      snapshot
    })
}

/// The packages whose resolved version set changed between two snapshots,
/// removed versions paired to added versions in sorted order.  A package
/// only added (a new transitive dependency) or only removed (a dropped
/// one) is not a bump and is left out — a version-change entry is what a
/// changelog reader understands a bump to be.
pub fn bumps_between(before: &Snapshot, after: &Snapshot) -> Vec<Bump> {
  before
    .iter()
    .filter_map(|(name, old_versions)| {
      after
        .get(name)
        .map(|new_versions| (name, old_versions, new_versions))
    })
    .flat_map(|(name, old_versions, new_versions)| {
      multiset_difference(old_versions, new_versions)
        .into_iter()
        .zip(multiset_difference(new_versions, old_versions))
        .map(|(from, to)| Bump {
          name: name.clone(),
          from,
          to,
        })
        .collect::<Vec<_>>()
    })
    .collect()
}

/// Items of `left` not matched one-for-one in `right`; both sides sorted.
fn multiset_difference(left: &[String], right: &[String]) -> Vec<String> {
  // A two-pointer walk over the sorted sides; imperative because the
  // right-side cursor advances conditionally across iterations, which no
  // combinator expresses without contortion.
  let mut difference = Vec::new();
  let mut right_index = 0;
  for item in left {
    while right_index < right.len() && right[right_index] < *item {
      right_index += 1;
    }
    if right_index < right.len() && right[right_index] == *item {
      right_index += 1;
    } else {
      difference.push(item.clone());
    }
  }
  difference
}

#[cfg(test)]
mod tests {
  use super::*;

  fn snapshot_of(entries: &[(&str, &[&str])]) -> Snapshot {
    entries
      .iter()
      .map(|(name, versions)| {
        (
          (*name).to_string(),
          versions.iter().map(|v| (*v).to_string()).collect(),
        )
      })
      .collect()
  }

  #[test]
  fn a_version_change_is_a_bump() {
    let before = snapshot_of(&[("serde", &["1.0.200"])]);
    let after = snapshot_of(&[("serde", &["1.0.210"])]);
    assert_eq!(
      bumps_between(&before, &after),
      vec![Bump {
        name: "serde".to_string(),
        from: "1.0.200".to_string(),
        to: "1.0.210".to_string(),
      }]
    );
  }

  #[test]
  fn an_unchanged_package_is_not_a_bump() {
    let snapshot = snapshot_of(&[("serde", &["1.0.200"])]);
    assert!(bumps_between(&snapshot, &snapshot).is_empty());
  }

  #[test]
  fn a_newly_added_package_is_not_a_bump() {
    let before = snapshot_of(&[]);
    let after = snapshot_of(&[("bitflags", &["2.6.0"])]);
    assert!(bumps_between(&before, &after).is_empty());
  }

  #[test]
  fn a_removed_package_is_not_a_bump() {
    let before = snapshot_of(&[("bitflags", &["2.6.0"])]);
    let after = snapshot_of(&[]);
    assert!(bumps_between(&before, &after).is_empty());
  }

  #[test]
  fn one_of_two_coexisting_versions_moving_is_one_bump() {
    let before = snapshot_of(&[("syn", &["1.0.109", "2.0.60"])]);
    let after = snapshot_of(&[("syn", &["1.0.109", "2.0.75"])]);
    assert_eq!(
      bumps_between(&before, &after),
      vec![Bump {
        name: "syn".to_string(),
        from: "2.0.60".to_string(),
        to: "2.0.75".to_string(),
      }]
    );
  }

  #[test]
  fn parses_a_lockfile_into_a_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Cargo.lock");
    std::fs::write(
      &path,
      r#"
        version = 4

        [[package]]
        name = "serde"
        version = "1.0.200"

        [[package]]
        name = "syn"
        version = "2.0.60"

        [[package]]
        name = "syn"
        version = "1.0.109"
      "#,
    )
    .unwrap();
    let snapshot = snapshot(&path).unwrap();
    assert_eq!(snapshot["serde"], vec!["1.0.200"]);
    assert_eq!(snapshot["syn"], vec!["1.0.109", "2.0.60"]);
  }
}
