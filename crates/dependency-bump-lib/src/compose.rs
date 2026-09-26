//! Changelog entry composition: pure text assembly, kept separate from the
//! subprocess that inserts it so the wording is unit-testable.

use crate::audit::Advisories;
use crate::lockfile::Bump;
use crate::runners::{self, RunnerBump};
use std::fmt;

/// The changelog heading a bump files under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
  Security,
  Maintenance,
}

impl fmt::Display for Heading {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(match self {
      Heading::Security => "Security",
      Heading::Maintenance => "Maintenance",
    })
  }
}

/// One composed changelog entry for a bump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
  pub heading: Heading,
  pub body: String,
}

/// Composes the entry for a bump: Security with the advisory ids appended
/// when the pre-update lockfile carried an advisory against the package,
/// Maintenance otherwise.
pub fn entry(bump: &Bump, advisories: &Advisories) -> Entry {
  advisories.get(&bump.name).map_or_else(
    || Entry {
      heading: Heading::Maintenance,
      body: format!("Bump {} from {} to {}", bump.name, bump.from, bump.to),
    },
    |ids| Entry {
      heading: Heading::Security,
      body: format!(
        "Bump {} from {} to {} ({})",
        bump.name,
        bump.from,
        bump.to,
        ids.join(", ")
      ),
    },
  )
}

/// Composes the entry for a runner-image advance.  Always Maintenance: a
/// runner image has no advisory database to classify it against.
pub fn runner_entry(bump: &RunnerBump) -> Entry {
  Entry {
    heading: Heading::Maintenance,
    body: format!(
      "Bump the {} from {} to {}",
      bump.variant,
      runners::release(bump.from),
      runners::release(bump.to)
    ),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::runners::Variant;

  fn bump() -> Bump {
    Bump {
      name: "time".to_string(),
      from: "0.3.20".to_string(),
      to: "0.3.36".to_string(),
    }
  }

  #[test]
  fn a_plain_bump_files_under_maintenance() {
    let composed = entry(&bump(), &Advisories::new());
    assert_eq!(composed.heading, Heading::Maintenance);
    assert_eq!(composed.body, "Bump time from 0.3.20 to 0.3.36");
  }

  #[test]
  fn an_advisory_hit_files_under_security_with_the_ids() {
    let advisories = Advisories::from([(
      "time".to_string(),
      vec!["RUSTSEC-2020-0071".to_string()],
    )]);
    let composed = entry(&bump(), &advisories);
    assert_eq!(composed.heading, Heading::Security);
    assert_eq!(
      composed.body,
      "Bump time from 0.3.20 to 0.3.36 (RUSTSEC-2020-0071)"
    );
  }

  #[test]
  fn a_runner_advance_files_under_maintenance() {
    let composed = runner_entry(&RunnerBump {
      variant: Variant::Arm,
      from: 24,
      to: 26,
    });
    assert_eq!(composed.heading, Heading::Maintenance);
    assert_eq!(
      composed.body,
      "Bump the GitHub-hosted Ubuntu arm runner image from 24.04 to 26.04"
    );
  }
}
