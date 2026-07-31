//! Advisory classification from `cargo audit --json`.
//!
//! The report is consulted against the pre-update lockfile: a package
//! under advisory whose bump then lands files that bump under Security
//! rather than Maintenance.  An audit failure must never block the bump —
//! the caller degrades to an empty set with a warning, and every bump
//! files under Maintenance.

use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;

/// Advisory ids per affected package name.
pub type Advisories = BTreeMap<String, Vec<String>>;

/// Why the advisory probe produced nothing; feeds the caller's warning and
/// never stops the run.
#[derive(Debug, Error)]
pub enum AuditProbeError {
  #[error("could not run cargo audit: {0}")]
  Spawn(#[from] std::io::Error),
  #[error("cargo audit emitted non-UTF-8 output: {0}")]
  Encoding(#[from] std::string::FromUtf8Error),
  #[error("could not parse the cargo audit report: {0}")]
  Parse(#[from] serde_json::Error),
}

#[derive(Debug, Default, Deserialize)]
struct Report {
  #[serde(default)]
  vulnerabilities: Vulnerabilities,
}

#[derive(Debug, Default, Deserialize)]
struct Vulnerabilities {
  #[serde(default)]
  list: Vec<Vulnerability>,
}

#[derive(Debug, Deserialize)]
struct Vulnerability {
  advisory: Advisory,
}

#[derive(Debug, Deserialize)]
struct Advisory {
  package: String,
  id: String,
}

/// Parses `cargo audit --json` output into the advisory set.
pub fn parse_advisories(json: &str) -> Result<Advisories, serde_json::Error> {
  serde_json::from_str::<Report>(json).map(|report| {
    report.vulnerabilities.list.into_iter().fold(
      Advisories::new(),
      |mut advisories, vulnerability| {
        advisories
          .entry(vulnerability.advisory.package)
          .or_default()
          .push(vulnerability.advisory.id);
        advisories
      },
    )
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn collects_advisories_per_package() {
    let json = r#"{
      "vulnerabilities": {
        "list": [
          { "advisory": { "package": "time", "id": "RUSTSEC-2020-0071" } },
          { "advisory": { "package": "time", "id": "RUSTSEC-2021-0072" } },
          { "advisory": { "package": "libc", "id": "RUSTSEC-2019-0001" } }
        ]
      }
    }"#;
    let advisories = parse_advisories(json).unwrap();
    assert_eq!(
      advisories["time"],
      vec!["RUSTSEC-2020-0071", "RUSTSEC-2021-0072"]
    );
    assert_eq!(advisories["libc"], vec!["RUSTSEC-2019-0001"]);
  }

  #[test]
  fn an_empty_report_yields_no_advisories() {
    assert!(parse_advisories(r#"{"vulnerabilities":{"list":[]}}"#)
      .unwrap()
      .is_empty());
  }

  #[test]
  fn a_report_missing_the_section_yields_no_advisories() {
    assert!(parse_advisories("{}").unwrap().is_empty());
  }

  #[test]
  fn malformed_json_is_an_error() {
    assert!(parse_advisories("not json").is_err());
  }
}
