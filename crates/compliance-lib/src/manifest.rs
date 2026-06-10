//! The declarative check manifest — `compliance-checks.toml`.
//!
//! Each `[[check]]` entry names a stable `id`, a human `description`, a `kind`
//! from the finite set the engine implements, and the parameters that kind
//! needs.  An optional `when_crates_contains` restricts a check to spawns whose
//! crate-role list contains a given substring (e.g. only run a server check on
//! spawns generated with a `server` crate).
//!
//! The manifest is parsed in two stages: TOML into [`RawCheck`] (all
//! kind-specific parameters optional), then validated into a typed [`Check`]
//! whose [`CheckKind`] carries exactly the parameters that kind requires.  The
//! second stage yields precise errors ("check 'x' of kind 'section-exists' is
//! missing 'section'") rather than opaque deserialization failures.

use crate::error::ComplianceError;
use serde::Deserialize;
use std::path::Path;

/// A validated check: applicability plus a fully-specified [`CheckKind`].
#[derive(Debug, Clone)]
pub struct Check {
  pub id: String,
  pub description: String,
  /// When set, the check only applies to spawns whose `args.crates` string
  /// contains this substring; others report `Skip`.
  pub when_crates_contains: Option<String>,
  /// When `Some(true)`, the check only applies to spawns marked public; others
  /// report `Skip`.  Used for the crates.io publish machinery.
  pub when_public: Option<bool>,
  pub kind: CheckKind,
}

/// The finite set of check kinds the engine knows how to run.  `target` /
/// `path` values are paths relative to a spawn's project root.
#[derive(Debug, Clone)]
pub enum CheckKind {
  /// A required file exists at `path`.
  FilePresent { path: String },
  /// The file at `path` exists and parses as JSON.
  JsonValid { path: String },
  /// The file at `target` exists and contains `contains` as a substring.
  FileContains { target: String, contains: String },
  /// Some `crates/**/Cargo.toml` enables the named foundation `feature`.
  FoundationFeature { feature: String },
  /// No stale `rust-template` literals remain (foundation references and the
  /// GitHub URL are expected and excluded).
  NoStaleLiteral,
  /// `target` (an org document) contains a heading whose text equals
  /// `section`.
  SectionExists { target: String, section: String },
  /// `target` contains `contains` as a substring after line-wrapped
  /// paragraphs are flattened; when `section` is set, the search is scoped to
  /// that section's body.
  MentionPresent {
    target: String,
    section: Option<String>,
    contains: String,
  },
  /// The foundation revision pinned in `Cargo.lock` equals the one pinned in
  /// `flake.lock` (the two dependency edges agree with each other).
  PinsAgree,
  /// Both foundation pins equal the template's current `HEAD` (the spawn is
  /// on the latest template).
  PinsCurrent,
  /// No file exists at `path` (the inverse of `file-present`).
  FileAbsent { path: String },
  /// The spawn's file at `path` is byte-for-byte identical to the template's
  /// canonical copy under `template/`.  Skips when either file is absent.
  FileMatchesTemplate { path: String },
  /// At least one file in the spawn matches `glob` (a `/`-separated pattern
  /// where `*` matches one path segment).
  GlobPresent { glob: String },
  /// `target` parses as JSON and the value at `pointer` (a dotted path,
  /// e.g. `a.b.0`) exists.  Skips when `target` is absent.
  JsonPathExists { target: String, pointer: String },
  /// `target` parses as JSON and the scalar at `pointer` equals `value`.
  JsonPathEquals {
    target: String,
    pointer: String,
    value: String,
  },
  /// `target` parses as JSON and the sequence at `pointer` contains an element
  /// equal to `value`.
  JsonSeqContains {
    target: String,
    pointer: String,
    value: String,
  },
  /// `target` parses as TOML and the value at `pointer` exists.
  TomlPathExists { target: String, pointer: String },
  /// `target` parses as TOML and the scalar at `pointer` equals `value`.
  TomlPathEquals {
    target: String,
    pointer: String,
    value: String,
  },
}

/// The TOML shape: every kind-specific field is optional and validated later.
#[derive(Debug, Deserialize)]
struct RawManifest {
  #[serde(default)]
  check: Vec<RawCheck>,
}

#[derive(Debug, Deserialize)]
struct RawCheck {
  id: String,
  description: String,
  kind: String,
  #[serde(default)]
  when_crates_contains: Option<String>,
  #[serde(default)]
  when_public: Option<bool>,
  #[serde(default)]
  target: Option<String>,
  #[serde(default)]
  path: Option<String>,
  #[serde(default)]
  section: Option<String>,
  #[serde(default)]
  contains: Option<String>,
  #[serde(default)]
  feature: Option<String>,
  #[serde(default)]
  glob: Option<String>,
  #[serde(default)]
  pointer: Option<String>,
  #[serde(default)]
  value: Option<String>,
}

/// Require a kind-specific parameter, naming the check and kind on absence.
fn require(
  id: &str,
  kind: &str,
  name: &str,
  value: Option<String>,
) -> Result<String, ComplianceError> {
  value.ok_or_else(|| ComplianceError::ManifestInvalid {
    id: id.to_string(),
    message: format!("kind '{kind}' requires parameter '{name}'"),
  })
}

impl RawCheck {
  fn validate(self) -> Result<Check, ComplianceError> {
    let RawCheck {
      id,
      description,
      kind,
      when_crates_contains,
      when_public,
      target,
      path,
      section,
      contains,
      feature,
      glob,
      pointer,
      value,
    } = self;

    let resolved = match kind.as_str() {
      "file-present" => CheckKind::FilePresent {
        path: require(&id, &kind, "path", path)?,
      },
      "json-valid" => CheckKind::JsonValid {
        path: require(&id, &kind, "path", path)?,
      },
      "file-contains" => CheckKind::FileContains {
        target: require(&id, &kind, "target", target)?,
        contains: require(&id, &kind, "contains", contains)?,
      },
      "foundation-feature" => CheckKind::FoundationFeature {
        feature: require(&id, &kind, "feature", feature)?,
      },
      "no-stale-literal" => CheckKind::NoStaleLiteral,
      "section-exists" => CheckKind::SectionExists {
        target: require(&id, &kind, "target", target)?,
        section: require(&id, &kind, "section", section)?,
      },
      "mention-present" => CheckKind::MentionPresent {
        target: require(&id, &kind, "target", target)?,
        section,
        contains: require(&id, &kind, "contains", contains)?,
      },
      "pins-agree" => CheckKind::PinsAgree,
      "pins-current" => CheckKind::PinsCurrent,
      "file-absent" => CheckKind::FileAbsent {
        path: require(&id, &kind, "path", path)?,
      },
      "file-matches-template" => CheckKind::FileMatchesTemplate {
        path: require(&id, &kind, "path", path)?,
      },
      "glob-present" => CheckKind::GlobPresent {
        glob: require(&id, &kind, "glob", glob)?,
      },
      "json-path-exists" => CheckKind::JsonPathExists {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
      },
      "json-path-equals" => CheckKind::JsonPathEquals {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
        value: require(&id, &kind, "value", value)?,
      },
      "json-seq-contains" => CheckKind::JsonSeqContains {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
        value: require(&id, &kind, "value", value)?,
      },
      "toml-path-exists" => CheckKind::TomlPathExists {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
      },
      "toml-path-equals" => CheckKind::TomlPathEquals {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
        value: require(&id, &kind, "value", value)?,
      },
      other => {
        return Err(ComplianceError::ManifestInvalid {
          id: id.clone(),
          message: format!("unknown kind '{other}'"),
        })
      }
    };

    Ok(Check {
      id,
      description,
      when_crates_contains,
      when_public,
      kind: resolved,
    })
  }
}

/// Load and validate every check in the manifest at `path`.
pub fn load(path: &Path) -> Result<Vec<Check>, ComplianceError> {
  let text = std::fs::read_to_string(path).map_err(|source| {
    ComplianceError::ManifestRead {
      path: path.to_path_buf(),
      source,
    }
  })?;
  let raw: RawManifest =
    toml::from_str(&text).map_err(|source| ComplianceError::ManifestParse {
      path: path.to_path_buf(),
      source,
    })?;
  raw.check.into_iter().map(RawCheck::validate).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn validates_a_section_check() {
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "section-exists"
            target = "llms.org"
            section = "Template compliance"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let check = raw.check.into_iter().next().unwrap().validate().unwrap();
    assert_eq!(check.id, "x");
    match check.kind {
      CheckKind::SectionExists { target, section } => {
        assert_eq!(target, "llms.org");
        assert_eq!(section, "Template compliance");
      }
      other => panic!("wrong kind: {other:?}"),
    }
  }

  #[test]
  fn missing_required_param_is_an_error() {
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "section-exists"
            target = "llms.org"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let error = raw
      .check
      .into_iter()
      .next()
      .unwrap()
      .validate()
      .unwrap_err();
    assert!(matches!(error, ComplianceError::ManifestInvalid { .. }));
  }

  #[test]
  fn unknown_kind_is_an_error() {
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "teleport"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let error = raw
      .check
      .into_iter()
      .next()
      .unwrap()
      .validate()
      .unwrap_err();
    assert!(matches!(error, ComplianceError::ManifestInvalid { .. }));
  }
}
