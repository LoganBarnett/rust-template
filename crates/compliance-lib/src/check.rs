//! Running a single check against a single spawn.
//!
//! Every check resolves to a [`Verdict`].  Outcomes are data, not control
//! flow: a failing or erroring check never aborts the run, so one broken spawn
//! cannot hide the state of the others.  The outcome variants serialize
//! directly into the JSON report.

use crate::manifest::{Check, CheckKind};
use crate::org;
use crate::pins;
use crate::provenance::Provenance;
use serde::Serialize;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A check's verdict: the result of running one check against one spawn.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Verdict {
  /// The check held.
  Pass,
  /// The check did not hold; `detail` says how.
  Fail { detail: String },
  /// The check does not apply to this spawn (wrong crate roles, a missing
  /// prerequisite that is legitimately optional, etc.).
  Skip { reason: String },
  /// The spawn opted out of this check via `compliance-ignores`.
  Ignored { reason: Option<String> },
  /// The check could not be evaluated (an unexpected I/O or tooling failure).
  Error { detail: String },
}

impl Verdict {
  /// Whether this outcome should make the overall run fail.
  pub fn is_failure(&self) -> bool {
    matches!(self, Verdict::Fail { .. } | Verdict::Error { .. })
  }
}

/// Everything a check needs to know about the spawn it runs against.
pub struct SpawnContext<'a> {
  /// The spawn's project root.
  pub dir: &'a Path,
  /// The comma-separated crate-role list from the spawn manifest.
  pub crates: &'a str,
  /// The spawn's parsed provenance (for `compliance-ignores`).
  pub provenance: &'a Provenance,
  /// The template's current `HEAD`, resolved once per run (shared), or the
  /// reason it could not be resolved.
  pub template_head: &'a Result<String, String>,
  /// The template checkout root; its `template/` subtree holds the canonical
  /// files that `file-matches-template` compares against.
  pub template_dir: &'a Path,
  /// Whether the spawn is marked public (gates the publish-machinery checks).
  pub public: bool,
}

/// Run `check` against the spawn described by `ctx`.
pub fn run_check(check: &Check, ctx: &SpawnContext) -> Verdict {
  // An explicit opt-out wins over everything else.
  if let Some(reason) = ctx.provenance.ignored_reason(&check.id) {
    return Verdict::Ignored { reason };
  }

  // Role-conditional checks skip on spawns that lack the role.
  if let Some(role) = &check.when_crates_contains {
    if !ctx.crates.contains(role.as_str()) {
      return Verdict::Skip {
        reason: format!(
          "crate roles \"{}\" do not include \"{role}\"",
          ctx.crates
        ),
      };
    }
  }

  // Public-only checks (the crates.io publish machinery) skip on private spawns.
  if check.when_public == Some(true) && !ctx.public {
    return Verdict::Skip {
      reason: "spawn is not public".to_string(),
    };
  }

  // Feature-conditional checks skip on spawns that do not enable the feature,
  // reusing the same crate-manifest scan the `foundation-feature` kind runs.
  if let Some(feature) = &check.when_foundation_feature {
    if !foundation_feature_present(ctx.dir, feature) {
      return Verdict::Skip {
        reason: format!("foundation feature \"{feature}\" not enabled"),
      };
    }
  }

  match &check.kind {
    CheckKind::FilePresent { path } => file_present(ctx.dir, path),
    CheckKind::JsonValid { path } => json_valid(ctx.dir, path),
    CheckKind::FileContains { target, contains } => {
      file_contains(ctx.dir, target, contains)
    }
    CheckKind::FoundationFeature { feature } => {
      foundation_feature(ctx.dir, feature)
    }
    CheckKind::NoStaleLiteral => no_stale_literal(ctx.dir),
    CheckKind::SectionExists { target, section } => {
      section_exists(ctx.dir, target, section)
    }
    CheckKind::MentionPresent {
      target,
      section,
      contains,
    } => mention_present(ctx.dir, target, section.as_deref(), contains),
    CheckKind::PinsAgree => pins_agree(ctx.dir),
    CheckKind::PinsCurrent => pins_current(ctx.dir, ctx.template_head),
    CheckKind::FileAbsent { path } => file_absent(ctx.dir, path),
    CheckKind::FileMatchesTemplate { path } => {
      file_matches_template(ctx.dir, ctx.template_dir, path)
    }
    CheckKind::GlobPresent { glob } => glob_present(ctx.dir, glob),
    CheckKind::JsonPathExists { target, pointer } => {
      structured_path(ctx.dir, target, pointer, &PathMatch::Exists, parse_json)
    }
    CheckKind::JsonPathEquals {
      target,
      pointer,
      value,
    } => structured_path(
      ctx.dir,
      target,
      pointer,
      &PathMatch::Equals(value),
      parse_json,
    ),
    CheckKind::JsonSeqContains {
      target,
      pointer,
      value,
    } => structured_path(
      ctx.dir,
      target,
      pointer,
      &PathMatch::SeqContains(value),
      parse_json,
    ),
    CheckKind::TomlPathExists { target, pointer } => {
      structured_path(ctx.dir, target, pointer, &PathMatch::Exists, parse_toml)
    }
    CheckKind::TomlPathEquals {
      target,
      pointer,
      value,
    } => structured_path(
      ctx.dir,
      target,
      pointer,
      &PathMatch::Equals(value),
      parse_toml,
    ),
    CheckKind::TomlSeqContains {
      target,
      pointer,
      value,
    } => structured_path(
      ctx.dir,
      target,
      pointer,
      &PathMatch::SeqContains(value),
      parse_toml,
    ),
    CheckKind::CrateTomlPathEquals { pointer, value } => {
      crate_toml_path_equals(ctx.dir, pointer, value)
    }
    CheckKind::YamlPathExists { target, pointer } => {
      yaml_path(ctx.dir, target, pointer, &PathMatch::Exists)
    }
    CheckKind::YamlPathEquals {
      target,
      pointer,
      value,
    } => yaml_path(ctx.dir, target, pointer, &PathMatch::Equals(value)),
    CheckKind::YamlPathContains {
      target,
      pointer,
      contains,
    } => yaml_path(ctx.dir, target, pointer, &PathMatch::Contains(contains)),
    CheckKind::YamlPathNotContains {
      target,
      pointer,
      contains,
    } => yaml_path(ctx.dir, target, pointer, &PathMatch::NotContains(contains)),
    CheckKind::JustfileRecipe { recipe } => justfile_recipe(ctx.dir, recipe),
    CheckKind::YamlSeqContains {
      target,
      pointer,
      value,
    } => yaml_path(ctx.dir, target, pointer, &PathMatch::SeqContains(value)),
    CheckKind::RustFnHasAttr {
      target,
      function,
      attr,
    } => rust_fn_has_attr(ctx.dir, target, function, attr),
    CheckKind::RustStructHasDerive {
      target,
      struct_name,
      derive,
    } => rust_struct_has_derive(ctx.dir, target, struct_name, derive),
    CheckKind::RustStructHasHelperAttr {
      target,
      struct_name,
      attr,
    } => rust_struct_has_helper_attr(ctx.dir, target, struct_name, attr),
    CheckKind::RustStructFieldAttrCount {
      target,
      struct_name,
      attr,
      contains,
      count,
    } => rust_struct_field_attr_count(
      ctx.dir,
      target,
      struct_name,
      attr,
      contains.as_deref(),
      *count,
    ),
    CheckKind::RustUseGlob { target, path } => {
      rust_use_glob(ctx.dir, target, path)
    }
    CheckKind::RustImplTraitFor {
      target,
      trait_name,
      self_ty,
    } => rust_impl_trait_for(ctx.dir, target, trait_name, self_ty),
    CheckKind::RustMethodChain {
      target,
      function,
      methods,
    } => rust_method_chain(ctx.dir, target, function, methods),
    CheckKind::DevShellEnv { shell, var, value } => {
      dev_shell_env(ctx.dir, shell.as_deref(), var, value)
    }
    CheckKind::DevShellPackage { shell, package } => {
      dev_shell_package(ctx.dir, shell.as_deref(), package)
    }
    CheckKind::FlakeOutputPresent {
      output,
      system,
      suffix,
      name,
    } => flake_output_present(
      ctx.dir,
      output,
      system.as_deref(),
      suffix.as_deref(),
      name.as_deref(),
    ),
    CheckKind::NixModuleOptionDefault {
      module,
      option,
      value,
    } => nix_module_option_default(
      ctx.dir,
      ctx.template_dir,
      module,
      option,
      value,
    ),
  }
}

// ── Per-kind implementations ─────────────────────────────────────────

fn file_present(dir: &Path, path: &str) -> Verdict {
  if dir.join(path).exists() {
    Verdict::Pass
  } else {
    Verdict::Fail {
      detail: format!("required file missing: {path}"),
    }
  }
}

fn json_valid(dir: &Path, path: &str) -> Verdict {
  match read_file(&dir.join(path)) {
    FileRead::Found(text) => {
      match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(_) => Verdict::Pass,
        Err(error) => Verdict::Fail {
          detail: format!("{path} is not valid JSON: {error}"),
        },
      }
    }
    FileRead::Missing => Verdict::Fail {
      detail: format!("{path} not found"),
    },
    FileRead::Error(detail) => Verdict::Error { detail },
  }
}

fn file_contains(dir: &Path, target: &str, needle: &str) -> Verdict {
  match read_file(&dir.join(target)) {
    FileRead::Found(text) if text.contains(needle) => Verdict::Pass,
    FileRead::Found(_) => Verdict::Fail {
      detail: format!("{target} does not contain \"{needle}\""),
    },
    FileRead::Missing => Verdict::Fail {
      detail: format!("{target} not found"),
    },
    FileRead::Error(detail) => Verdict::Error { detail },
  }
}

fn foundation_feature(dir: &Path, feature: &str) -> Verdict {
  if foundation_feature_present(dir, feature) {
    Verdict::Pass
  } else {
    Verdict::Fail {
      detail: format!("no crate enables the foundation \"{feature}\" feature"),
    }
  }
}

fn no_stale_literal(dir: &Path) -> Verdict {
  let offenders = stale_literals(dir);
  if offenders.is_empty() {
    Verdict::Pass
  } else {
    Verdict::Fail {
      detail: format!(
        "stale rust-template literals in: {}",
        offenders.join(", ")
      ),
    }
  }
}

fn section_exists(dir: &Path, target: &str, section: &[String]) -> Verdict {
  match read_file(&dir.join(target)) {
    FileRead::Found(text) if org::section_exists(&text, section) => {
      Verdict::Pass
    }
    FileRead::Found(_) => Verdict::Fail {
      detail: format!(
        "section \"{}\" not found in {target}",
        section.join(" > ")
      ),
    },
    FileRead::Missing => Verdict::Fail {
      detail: format!("{target} not found"),
    },
    FileRead::Error(detail) => Verdict::Error { detail },
  }
}

fn mention_present(
  dir: &Path,
  target: &str,
  section: Option<&[String]>,
  needle: &str,
) -> Verdict {
  match read_file(&dir.join(target)) {
    FileRead::Found(text) => match org::mention_present(&text, section, needle)
    {
      Ok(true) => Verdict::Pass,
      Ok(false) => Verdict::Fail {
        detail: section.map_or_else(
          || format!("\"{needle}\" not found in {target}"),
          |name| {
            format!("\"{needle}\" not found in {target} § {}", name.join(" > "))
          },
        ),
      },
      Err(reason) => Verdict::Fail {
        detail: format!("{reason} in {target}"),
      },
    },
    FileRead::Missing => Verdict::Fail {
      detail: format!("{target} not found"),
    },
    FileRead::Error(detail) => Verdict::Error { detail },
  }
}

fn pins_agree(dir: &Path) -> Verdict {
  match pins(dir) {
    Pins::Both { cargo, flake } if cargo == flake => Verdict::Pass,
    Pins::Both { cargo, flake } => Verdict::Fail {
      detail: format!("Cargo.lock pins {cargo}; flake.lock pins {flake}"),
    },
    Pins::Skip(reason) => Verdict::Skip { reason },
    Pins::Error(detail) => Verdict::Error { detail },
  }
}

fn pins_current(dir: &Path, template_head: &Result<String, String>) -> Verdict {
  let (cargo, flake) = match pins(dir) {
    Pins::Both { cargo, flake } => (cargo, flake),
    Pins::Skip(reason) => return Verdict::Skip { reason },
    Pins::Error(detail) => return Verdict::Error { detail },
  };
  let head = match template_head {
    Ok(head) => head,
    Err(reason) => {
      return Verdict::Error {
        detail: format!("template HEAD unavailable: {reason}"),
      }
    }
  };
  if &cargo == head && &flake == head {
    Verdict::Pass
  } else {
    Verdict::Fail {
      detail: format!("Cargo {cargo} / flake {flake} differ from HEAD {head}"),
    }
  }
}

fn file_absent(dir: &Path, path: &str) -> Verdict {
  if dir.join(path).exists() {
    Verdict::Fail {
      detail: format!("file should not exist: {path}"),
    }
  } else {
    Verdict::Pass
  }
}

fn file_matches_template(
  dir: &Path,
  template_dir: &Path,
  path: &str,
) -> Verdict {
  let spawn = match read_file(&dir.join(path)) {
    FileRead::Found(text) => text,
    FileRead::Missing => {
      return Verdict::Skip {
        reason: format!("{path} not present"),
      }
    }
    FileRead::Error(detail) => return Verdict::Error { detail },
  };
  let canonical_path = template_dir.join("template").join(path);
  let canonical = match read_file(&canonical_path) {
    FileRead::Found(text) => text,
    FileRead::Missing => {
      return Verdict::Skip {
        reason: format!(
          "no canonical template file at {}",
          canonical_path.display()
        ),
      }
    }
    FileRead::Error(detail) => return Verdict::Error { detail },
  };
  if spawn == canonical {
    Verdict::Pass
  } else {
    Verdict::Fail {
      detail: format!("{path} differs from the template's canonical copy"),
    }
  }
}

fn glob_present(dir: &Path, glob: &str) -> Verdict {
  let mut files = Vec::new();
  walk_files(dir, &mut files);
  let matched = files.iter().any(|path| {
    path
      .strip_prefix(dir)
      .ok()
      .and_then(|rel| rel.to_str())
      .is_some_and(|rel| glob_matches(glob, rel))
  });
  if matched {
    Verdict::Pass
  } else {
    Verdict::Fail {
      detail: format!("no file matches \"{glob}\""),
    }
  }
}

/// Match a `/`-separated glob where each segment may use `*` as a wildcard for
/// any run of characters within that segment.  Segment counts must match.
fn glob_matches(pattern: &str, path: &str) -> bool {
  let pattern_segments: Vec<&str> = pattern.split('/').collect();
  let path_segments: Vec<&str> = path.split('/').collect();
  pattern_segments.len() == path_segments.len()
    && pattern_segments
      .iter()
      .zip(&path_segments)
      .all(|(pattern, segment)| segment_matches(pattern, segment))
}

fn segment_matches(pattern: &str, segment: &str) -> bool {
  match pattern.split_once('*') {
    None => pattern == segment,
    Some((prefix, suffix)) => {
      segment.len() >= prefix.len() + suffix.len()
        && segment.starts_with(prefix)
        && segment.ends_with(suffix)
    }
  }
}

/// How a structured-document check matches the value at its pointer.
enum PathMatch<'a> {
  /// The pointer resolves to any value.
  Exists,
  /// The pointer resolves to a scalar equal to this string.
  Equals(&'a str),
  /// The pointer resolves to a scalar containing this as a substring.
  Contains(&'a str),
  /// The pointer resolves to a scalar that does NOT contain this substring —
  /// or does not resolve at all.  Forbids a specific value without requiring
  /// the pointer to be present.
  NotContains(&'a str),
  /// The pointer resolves to a sequence containing a scalar equal to this.
  SeqContains(&'a str),
}

/// A value found at a pointer, reduced to what the matchers need so JSON/TOML
/// and YAML can share one matcher: whether it exists, its scalar form (if a
/// scalar), and its sequence's scalars (if a sequence).
struct Resolved {
  exists: bool,
  scalar: Option<String>,
  seq: Option<Vec<String>>,
}

impl Resolved {
  fn absent() -> Self {
    Resolved {
      exists: false,
      scalar: None,
      seq: None,
    }
  }
}

/// Render the verdict for `matcher` against an already-resolved value.  This is
/// the pure decision layer: the callers do the I/O and reduce a document to a
/// [`Resolved`]; this turns (what was wanted, what was found) into a [`Verdict`]
/// with no further reads.  `target` / `pointer` only feed the failure messages.
fn verdict(
  target: &str,
  pointer: &str,
  matcher: &PathMatch,
  resolved: &Resolved,
) -> Verdict {
  match matcher {
    PathMatch::Exists if resolved.exists => Verdict::Pass,
    PathMatch::Exists => Verdict::Fail {
      detail: format!("{target}: no value at \"{pointer}\""),
    },
    PathMatch::Equals(expected) => match &resolved.scalar {
      Some(actual) if actual == expected => Verdict::Pass,
      Some(actual) => Verdict::Fail {
        detail: format!(
          "{target}: \"{pointer}\" is \"{actual}\", expected \"{expected}\""
        ),
      },
      None => Verdict::Fail {
        detail: format!("{target}: no scalar at \"{pointer}\""),
      },
    },
    PathMatch::Contains(needle) => match &resolved.scalar {
      Some(actual) if actual.contains(needle) => Verdict::Pass,
      Some(actual) => Verdict::Fail {
        detail: format!(
          "{target}: \"{pointer}\" is \"{actual}\", missing \"{needle}\""
        ),
      },
      None => Verdict::Fail {
        detail: format!("{target}: no scalar at \"{pointer}\""),
      },
    },
    // An absent scalar passes: the point is to forbid a specific value, not to
    // require the pointer.  A present scalar carrying the needle is the one
    // failure — e.g. a spawn still holding a guard the reusable now owns.
    PathMatch::NotContains(needle) => match &resolved.scalar {
      Some(actual) if actual.contains(needle) => Verdict::Fail {
        detail: format!("{target}: \"{pointer}\" still contains \"{needle}\""),
      },
      _ => Verdict::Pass,
    },
    PathMatch::SeqContains(expected) => match &resolved.seq {
      Some(items) if items.iter().any(|item| item == expected) => Verdict::Pass,
      Some(_) => Verdict::Fail {
        detail: format!(
          "{target}: sequence at \"{pointer}\" has no \"{expected}\""
        ),
      },
      None => Verdict::Fail {
        detail: format!("{target}: no sequence at \"{pointer}\""),
      },
    },
  }
}

fn parse_json(text: &str) -> Result<serde_json::Value, String> {
  serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))
}

fn parse_toml(text: &str) -> Result<serde_json::Value, String> {
  let value: toml::Value =
    toml::from_str(text).map_err(|error| format!("invalid TOML: {error}"))?;
  serde_json::to_value(value).map_err(|error| format!("TOML to JSON: {error}"))
}

/// Read `target`, parse it via `parse`, navigate to `pointer`, and apply
/// `matcher`.  A missing target skips — its existence is a separate concern,
/// checked (where required) by a `file-present` check.
fn structured_path(
  dir: &Path,
  target: &str,
  pointer: &str,
  matcher: &PathMatch,
  parse: fn(&str) -> Result<serde_json::Value, String>,
) -> Verdict {
  let text = match read_file(&dir.join(target)) {
    FileRead::Found(text) => text,
    FileRead::Missing => {
      return Verdict::Skip {
        reason: format!("{target} not present"),
      }
    }
    FileRead::Error(detail) => return Verdict::Error { detail },
  };
  let value = match parse(&text) {
    Ok(value) => value,
    Err(detail) => {
      return Verdict::Fail {
        detail: format!("{target}: {detail}"),
      }
    }
  };
  verdict(target, pointer, matcher, &resolve_json(&value, pointer))
}

/// Assert that the scalar at `pointer` equals `value` in *every*
/// `crates/*/Cargo.toml`.  The crate manifests are discovered rather than
/// named, so the check holds archetype and custom-named crates to the same
/// rule (e.g. every crate deferring to a workspace dependency).  Skips when the
/// spawn has no crate manifests; fails naming each crate that diverges.
fn crate_toml_path_equals(dir: &Path, pointer: &str, value: &str) -> Verdict {
  let mut files = Vec::new();
  walk_files(dir, &mut files);
  let mut manifests: Vec<PathBuf> = files
    .into_iter()
    .filter(|path| {
      path
        .strip_prefix(dir)
        .ok()
        .and_then(|rel| rel.to_str())
        .is_some_and(|rel| glob_matches("crates/*/Cargo.toml", rel))
    })
    .collect();
  manifests.sort();
  if manifests.is_empty() {
    return Verdict::Skip {
      reason: "no crates/*/Cargo.toml present".to_string(),
    };
  }

  // Each manifest yields `Ok(None)` when it satisfies the rule, `Ok(Some(why))`
  // when it diverges, or `Err(detail)` for a hard read error.  Collecting into a
  // `Result` short-circuits the whole check to an `Error` on the first `Err`.
  manifests
    .iter()
    .map(|manifest| crate_pointer_divergence(dir, manifest, pointer, value))
    .collect::<Result<Vec<_>, _>>()
    .map_or_else(
      |detail| Verdict::Error { detail },
      |results| {
        let failures: Vec<String> = results.into_iter().flatten().collect();
        if failures.is_empty() {
          Verdict::Pass
        } else {
          Verdict::Fail {
            detail: format!(
              "\"{pointer}\" != \"{value}\": {}",
              failures.join("; ")
            ),
          }
        }
      },
    )
}

/// One crate manifest's contribution to [`crate_toml_path_equals`]: `Ok(None)`
/// when its scalar at `pointer` equals `value`, `Ok(Some(reason))` when it
/// diverges, or `Err(detail)` for a hard read error that aborts the check.
fn crate_pointer_divergence(
  dir: &Path,
  manifest: &Path,
  pointer: &str,
  value: &str,
) -> Result<Option<String>, String> {
  let rel = manifest.strip_prefix(dir).unwrap_or(manifest).display();
  match read_file(manifest) {
    FileRead::Found(text) => Ok(
      match parse_toml(&text).map(|json| resolve_json(&json, pointer).scalar) {
        Ok(Some(actual)) if actual == value => None,
        Ok(Some(actual)) => {
          Some(format!("{rel}: \"{pointer}\" is \"{actual}\""))
        }
        Ok(None) => Some(format!("{rel}: no scalar at \"{pointer}\"")),
        Err(detail) => Some(format!("{rel}: {detail}")),
      },
    ),
    FileRead::Missing => Ok(Some(format!("{rel}: not found"))),
    FileRead::Error(detail) => Err(detail),
  }
}

fn resolve_json(value: &serde_json::Value, pointer: &str) -> Resolved {
  navigate(value, pointer).map_or_else(Resolved::absent, |found| Resolved {
    exists: true,
    scalar: scalar_to_string(found),
    seq: found
      .as_array()
      .map(|items| items.iter().filter_map(scalar_to_string).collect()),
  })
}

/// Like [`structured_path`] but for YAML, navigated with yaml-rust2's value
/// tree (which keeps scalar keys raw, so the `on:` trigger key is handled).
fn yaml_path(
  dir: &Path,
  target: &str,
  pointer: &str,
  matcher: &PathMatch,
) -> Verdict {
  let text = match read_file(&dir.join(target)) {
    FileRead::Found(text) => text,
    FileRead::Missing => {
      return Verdict::Skip {
        reason: format!("{target} not present"),
      }
    }
    FileRead::Error(detail) => return Verdict::Error { detail },
  };
  let docs = match yaml_rust2::YamlLoader::load_from_str(&text) {
    Ok(docs) => docs,
    Err(error) => {
      return Verdict::Fail {
        detail: format!("{target}: invalid YAML: {error}"),
      }
    }
  };
  let Some(doc) = docs.first() else {
    return Verdict::Fail {
      detail: format!("{target}: empty YAML"),
    };
  };
  verdict(target, pointer, matcher, &resolve_yaml(doc, pointer))
}

fn resolve_yaml(value: &yaml_rust2::Yaml, pointer: &str) -> Resolved {
  yaml_navigate(value, pointer).map_or_else(Resolved::absent, |found| {
    Resolved {
      exists: true,
      scalar: yaml_scalar(found),
      seq: match found {
        yaml_rust2::Yaml::Array(items) => {
          Some(items.iter().filter_map(yaml_scalar).collect())
        }
        _ => None,
      },
    }
  })
}

/// Navigate a YAML value by a dotted pointer, descending mapping keys and
/// array indices.  A `on`/`off`/`yes`/`no` segment also matches the boolean
/// key a YAML-1.1 parser would have produced.
fn yaml_navigate<'a>(
  value: &'a yaml_rust2::Yaml,
  pointer: &str,
) -> Option<&'a yaml_rust2::Yaml> {
  let mut current = value;
  for segment in pointer.split('.') {
    current = yaml_child(current, segment)?;
  }
  Some(current)
}

fn yaml_child<'a>(
  value: &'a yaml_rust2::Yaml,
  segment: &str,
) -> Option<&'a yaml_rust2::Yaml> {
  match value {
    yaml_rust2::Yaml::Hash(map) => map
      .get(&yaml_rust2::Yaml::String(segment.to_string()))
      .or_else(|| {
        bool_alias(segment)
          .and_then(|boolean| map.get(&yaml_rust2::Yaml::Boolean(boolean)))
      }),
    yaml_rust2::Yaml::Array(items) => segment
      .parse::<usize>()
      .ok()
      .and_then(|index| items.get(index)),
    _ => None,
  }
}

fn bool_alias(segment: &str) -> Option<bool> {
  match segment {
    "on" | "yes" => Some(true),
    "off" | "no" => Some(false),
    _ => None,
  }
}

fn yaml_scalar(value: &yaml_rust2::Yaml) -> Option<String> {
  match value {
    yaml_rust2::Yaml::String(string) => Some(string.clone()),
    yaml_rust2::Yaml::Integer(integer) => Some(integer.to_string()),
    yaml_rust2::Yaml::Boolean(boolean) => Some(boolean.to_string()),
    yaml_rust2::Yaml::Real(real) => Some(real.clone()),
    _ => None,
  }
}

/// Navigate a JSON value by a dotted pointer (`a.b.0`), descending object keys
/// and array indices.
fn navigate<'a>(
  value: &'a serde_json::Value,
  pointer: &str,
) -> Option<&'a serde_json::Value> {
  let mut current = value;
  for segment in pointer.split('.') {
    current = match current {
      serde_json::Value::Object(map) => map.get(segment)?,
      serde_json::Value::Array(items) => {
        items.get(segment.parse::<usize>().ok()?)?
      }
      _ => return None,
    };
  }
  Some(current)
}

/// The string form of a JSON scalar (string, number, bool); `None` for
/// objects, arrays, and null.
fn scalar_to_string(value: &serde_json::Value) -> Option<String> {
  match value {
    serde_json::Value::String(string) => Some(string.clone()),
    serde_json::Value::Number(number) => Some(number.to_string()),
    serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
    _ => None,
  }
}

// ── Rust AST helpers ─────────────────────────────────────────────────

/// Read and parse `target` as a Rust file, applying `check` to the AST.  A
/// missing target skips; a parse failure fails.
fn with_rust(
  dir: &Path,
  target: &str,
  check: impl FnOnce(&syn::File) -> Verdict,
) -> Verdict {
  match read_file(&dir.join(target)) {
    FileRead::Found(text) => match syn::parse_file(&text) {
      Ok(file) => check(&file),
      Err(error) => Verdict::Fail {
        detail: format!("{target}: invalid Rust: {error}"),
      },
    },
    FileRead::Missing => Verdict::Skip {
      reason: format!("{target} not present"),
    },
    FileRead::Error(detail) => Verdict::Error { detail },
  }
}

fn find_fn<'a>(file: &'a syn::File, name: &str) -> Option<&'a syn::ItemFn> {
  file.items.iter().find_map(|item| match item {
    syn::Item::Fn(item) if item.sig.ident == name => Some(item),
    _ => None,
  })
}

fn find_struct<'a>(
  file: &'a syn::File,
  name: &str,
) -> Option<&'a syn::ItemStruct> {
  file.items.iter().find_map(|item| match item {
    syn::Item::Struct(item) if item.ident == name => Some(item),
    _ => None,
  })
}

/// The last segment of an attribute's path (`merge_config` for
/// `#[merge_config(...)]`, `foundation_main` for `#[foundation_main]`).
fn attr_last_segment(attr: &syn::Attribute) -> Option<String> {
  attr.path().segments.last().map(|seg| seg.ident.to_string())
}

fn path_last_segment(path: &syn::Path) -> Option<String> {
  path.segments.last().map(|seg| seg.ident.to_string())
}

/// Whether an attribute's nested token text contains `needle` — the
/// leaf-level substring escape hatch, e.g. `common` in `#[merge_config(common)]`.
fn attr_tokens_contain(attr: &syn::Attribute, needle: &str) -> bool {
  match &attr.meta {
    syn::Meta::List(list) => list.tokens.to_string().contains(needle),
    _ => false,
  }
}

fn rust_fn_has_attr(
  dir: &Path,
  target: &str,
  function: &str,
  attr: &str,
) -> Verdict {
  with_rust(dir, target, |file| match find_fn(file, function) {
    None => Verdict::Fail {
      detail: format!("{target}: no fn `{function}`"),
    },
    Some(item)
      if item
        .attrs
        .iter()
        .any(|a| attr_last_segment(a).as_deref() == Some(attr)) =>
    {
      Verdict::Pass
    }
    Some(_) => Verdict::Fail {
      detail: format!("{target}: fn `{function}` is not annotated #[{attr}]"),
    },
  })
}

fn rust_struct_has_derive(
  dir: &Path,
  target: &str,
  struct_name: &str,
  derive: &str,
) -> Verdict {
  with_rust(dir, target, |file| match find_struct(file, struct_name) {
    None => Verdict::Fail {
      detail: format!("{target}: no struct `{struct_name}`"),
    },
    Some(item) if struct_has_derive(item, derive) => Verdict::Pass,
    Some(_) => Verdict::Fail {
      detail: format!(
        "{target}: struct `{struct_name}` does not derive {derive}"
      ),
    },
  })
}

fn struct_has_derive(item: &syn::ItemStruct, derive: &str) -> bool {
  item
    .attrs
    .iter()
    .filter(|a| a.path().is_ident("derive"))
    .any(|a| {
      a.parse_args_with(
        syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
      )
      .map(|paths| {
        paths
          .iter()
          .any(|p| path_last_segment(p).as_deref() == Some(derive))
      })
      .unwrap_or(false)
    })
}

fn rust_struct_has_helper_attr(
  dir: &Path,
  target: &str,
  struct_name: &str,
  attr: &str,
) -> Verdict {
  with_rust(dir, target, |file| match find_struct(file, struct_name) {
    None => Verdict::Fail {
      detail: format!("{target}: no struct `{struct_name}`"),
    },
    Some(item)
      if item
        .attrs
        .iter()
        .any(|a| attr_last_segment(a).as_deref() == Some(attr)) =>
    {
      Verdict::Pass
    }
    Some(_) => Verdict::Fail {
      detail: format!("{target}: struct `{struct_name}` lacks #[{attr}]"),
    },
  })
}

fn rust_struct_field_attr_count(
  dir: &Path,
  target: &str,
  struct_name: &str,
  attr: &str,
  contains: Option<&str>,
  count: u32,
) -> Verdict {
  with_rust(dir, target, |file| {
    find_struct(file, struct_name).map_or_else(
      || Verdict::Fail {
        detail: format!("{target}: no struct `{struct_name}`"),
      },
      |item| {
        let actual = item
          .fields
          .iter()
          .filter(|field| {
            field.attrs.iter().any(|a| {
              attr_last_segment(a).as_deref() == Some(attr)
                && contains.is_none_or(|needle| attr_tokens_contain(a, needle))
            })
          })
          .count() as u32;
        if actual == count {
          Verdict::Pass
        } else {
          Verdict::Fail {
            detail: format!(
              "{target}: struct `{struct_name}` has {actual} matching fields, expected {count}"
            ),
          }
        }
      },
    )
  })
}

fn rust_use_glob(dir: &Path, target: &str, path: &str) -> Verdict {
  with_rust(dir, target, |file| {
    if file.items.iter().any(|item| use_glob_matches(item, path)) {
      Verdict::Pass
    } else {
      Verdict::Fail {
        detail: format!("{target}: no `use {path}::*` import"),
      }
    }
  })
}

fn use_glob_matches(item: &syn::Item, path: &str) -> bool {
  let syn::Item::Use(item) = item else {
    return false;
  };
  use_tree_glob_path(&item.tree).is_some_and(|found| found == path)
}

/// If a use-tree ends in a glob (`a::b::*`), return the `a::b` prefix.
fn use_tree_glob_path(tree: &syn::UseTree) -> Option<String> {
  match tree {
    syn::UseTree::Path(path) => use_tree_glob_path(&path.tree).map(|rest| {
      if rest.is_empty() {
        path.ident.to_string()
      } else {
        format!("{}::{}", path.ident, rest)
      }
    }),
    syn::UseTree::Glob(_) => Some(String::new()),
    _ => None,
  }
}

fn rust_impl_trait_for(
  dir: &Path,
  target: &str,
  trait_name: &str,
  self_ty: &str,
) -> Verdict {
  with_rust(dir, target, |file| {
    if file
      .items
      .iter()
      .any(|item| impl_matches(item, trait_name, self_ty))
    {
      Verdict::Pass
    } else {
      Verdict::Fail {
        detail: format!("{target}: no `impl {trait_name} for {self_ty}`"),
      }
    }
  })
}

fn impl_matches(item: &syn::Item, trait_name: &str, self_ty: &str) -> bool {
  let syn::Item::Impl(item) = item else {
    return false;
  };
  let Some((_, trait_path, _)) = &item.trait_ else {
    return false;
  };
  path_last_segment(trait_path).as_deref() == Some(trait_name)
    && type_last_segment(&item.self_ty).as_deref() == Some(self_ty)
}

fn type_last_segment(ty: &syn::Type) -> Option<String> {
  match ty {
    syn::Type::Path(path) => path_last_segment(&path.path),
    _ => None,
  }
}

fn rust_method_chain(
  dir: &Path,
  target: &str,
  function: &str,
  methods: &[String],
) -> Verdict {
  with_rust(dir, target, |file| {
    find_fn(file, function).map_or_else(
      || Verdict::Fail {
        detail: format!("{target}: no fn `{function}`"),
      },
      |item| {
        let mut visitor = MethodVisitor::default();
        syn::visit::visit_item_fn(&mut visitor, item);
        let missing: Vec<&str> = methods
          .iter()
          .map(String::as_str)
          .filter(|method| !visitor.methods.contains(*method))
          .collect();
        if missing.is_empty() {
          Verdict::Pass
        } else {
          Verdict::Fail {
            detail: format!(
              "{target}: fn `{function}` does not call: {}",
              missing.join(", ")
            ),
          }
        }
      },
    )
  })
}

#[derive(Default)]
struct MethodVisitor {
  methods: std::collections::HashSet<String>,
}

impl<'ast> syn::visit::Visit<'ast> for MethodVisitor {
  fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
    self.methods.insert(call.method.to_string());
    syn::visit::visit_expr_method_call(self, call);
  }
}

// ── Nix devShell probe ───────────────────────────────────────────────

/// `nix eval` the string at `devShells.<system>.<shell>.<var>` (the default
/// devShell when `shell` is `None`) and compare it to `expected`.  mkShell
/// turns a plain attribute into the environment variable the shell exports,
/// so the evaluated value is exactly what the shell would print at runtime —
/// read here without realizing the shell's (possibly heavy) closure the way
/// `nix develop` would.  Pass on a match; Fail when it differs or the
/// attribute does not evaluate (marker absent or flake broken); Error when
/// nix cannot be run.
fn dev_shell_env(
  dir: &Path,
  shell: Option<&str>,
  var: &str,
  expected: &str,
) -> Verdict {
  let attr =
    format!("devShells.{}.{}.{var}", nix_system(), shell.unwrap_or("default"));
  // `nix eval --raw` prints the attribute's string with no quoting; every
  // argument is long-form.
  let output = match Command::new("nix")
    .args([
      "eval",
      "--raw",
      "--extra-experimental-features",
      "nix-command flakes",
      &format!("{}#{attr}", dir.display()),
    ])
    .output()
  {
    Ok(output) => output,
    Err(error) => {
      return Verdict::Error {
        detail: format!("could not run nix eval for {attr}: {error}"),
      }
    }
  };
  if !output.status.success() {
    return Verdict::Fail {
      detail: format!(
        "{attr} did not evaluate (marker absent or flake broken): {}",
        String::from_utf8_lossy(&output.stderr).trim()
      ),
    };
  }
  let printed = String::from_utf8_lossy(&output.stdout);
  let found = printed.trim();
  if found == expected {
    Verdict::Pass
  } else {
    Verdict::Fail {
      detail: format!("{attr} is \"{found}\", expected \"{expected}\""),
    }
  }
}

/// `nix eval` the spawn's devShell (default or `shell`) and confirm one of its
/// build inputs is the package whose derivation name is `package`.  Reading the
/// resolved shell's `buildInputs`/`nativeBuildInputs` — rather than grepping
/// flake.nix — means only a package actually pulled into the shell satisfies
/// it, and (like dev_shell_env) evaluating the inputs' names never realizes the
/// shell's heavy closure the way `nix develop` would.  Pass when found; Fail
/// when absent or the shell does not evaluate; Error when nix cannot be run.
fn dev_shell_package(
  dir: &Path,
  shell: Option<&str>,
  package: &str,
) -> Verdict {
  let attr =
    format!("devShells.{}.{}", nix_system(), shell.unwrap_or("default"));
  // `package` is a manifest-validated bare identifier (require_flake_ident), so
  // interpolating it into the predicate stays injection-free.  mkShell routes
  // `packages` into nativeBuildInputs and `buildInputs` into buildInputs, so
  // both lists are scanned; a package's clean name is `pname` when it carries a
  // version and `name` otherwise (a writeShellApplication has only `name`).
  let apply = format!(
    "shell: builtins.any (p: (p.pname or p.name or \"\") == \"{package}\") \
     ((shell.buildInputs or []) ++ (shell.nativeBuildInputs or []))"
  );
  // Every argument is long-form; the applied function prints a bare boolean.
  let evaluated = match Command::new("nix")
    .args([
      "eval",
      "--extra-experimental-features",
      "nix-command flakes",
      &format!("{}#{attr}", dir.display()),
      "--apply",
      &apply,
    ])
    .output()
  {
    Ok(evaluated) => evaluated,
    Err(error) => {
      return Verdict::Error {
        detail: format!("could not run nix eval for {attr}: {error}"),
      }
    }
  };
  if !evaluated.status.success() {
    return Verdict::Fail {
      detail: format!(
        "{attr} did not evaluate (shell absent or flake broken): {}",
        String::from_utf8_lossy(&evaluated.stderr).trim()
      ),
    };
  }
  match String::from_utf8_lossy(&evaluated.stdout).trim() {
    "true" => Verdict::Pass,
    "false" => Verdict::Fail {
      detail: format!("{attr} has no build input named \"{package}\""),
    },
    other => Verdict::Error {
      detail: format!("{attr} check returned unexpected \"{other}\""),
    },
  }
}

/// Run `just --summary` in the spawn and confirm it lists a recipe named
/// `recipe`.  Querying just's own recipe list rather than searching the file
/// text means a mention in a comment does not satisfy the check — only a real
/// recipe does.  A spawn without a justfile skips.
fn justfile_recipe(dir: &Path, recipe: &str) -> Verdict {
  let justfile = dir.join("justfile");
  if !justfile.exists() {
    return Verdict::Skip {
      reason: "justfile not present".to_string(),
    };
  }
  // `--summary` prints the recipe names space-separated; `--justfile` and
  // `--working-directory` point just at the spawn rather than the process's own
  // directory.  Every argument is long-form.
  let output = match Command::new("just")
    .arg("--justfile")
    .arg(&justfile)
    .arg("--working-directory")
    .arg(dir)
    .arg("--summary")
    .output()
  {
    Ok(output) => output,
    Err(error) => {
      return Verdict::Error {
        detail: format!("could not run just --summary: {error}"),
      }
    }
  };
  if !output.status.success() {
    return Verdict::Fail {
      detail: format!(
        "just --summary failed (unparseable justfile?): {}",
        String::from_utf8_lossy(&output.stderr).trim()
      ),
    };
  }
  let listed = String::from_utf8_lossy(&output.stdout);
  if listed.split_whitespace().any(|name| name == recipe) {
    Verdict::Pass
  } else {
    Verdict::Fail {
      detail: format!(
        "justfile has no recipe \"{recipe}\"; recipes: {}",
        listed.trim()
      ),
    }
  }
}

/// `nix eval` the flake output attrset `<output>.<system>` (the host double when
/// `system` is `None`) and confirm it exposes a required attribute: any name
/// ending with `suffix` (for a per-crate package like `<crate>-x86_64-windows`,
/// whose prefix varies per spawn), or the exact `name` (for a fixed output like
/// a flake check).  Exactly one selector is set; the caller's manifest
/// validation guarantees that.  The `--apply` function returns a Nix boolean,
/// which `nix eval` prints as `true`/`false` — evaluating the attrset's names
/// without realizing any derivation.  Pass on `true`; Fail when the attribute is
/// absent or the output does not evaluate; Error when nix cannot be run.
fn flake_output_present(
  dir: &Path,
  output: &str,
  system: Option<&str>,
  suffix: Option<&str>,
  name: Option<&str>,
) -> Verdict {
  let system = system.map_or_else(nix_system, str::to_string);
  let attr = format!("{output}.{system}");
  // Both selectors interpolate a manifest-validated bare identifier (see
  // require_flake_ident), so the expression stays injection-free.  Suffix
  // matching uses substring comparison rather than a regex to sidestep any
  // metacharacter ambiguity.
  let apply = match (suffix, name) {
    (Some(suffix), _) => format!(
      "xs: builtins.any (n: let nl = builtins.stringLength n; \
       sl = builtins.stringLength \"{suffix}\"; in \
       nl >= sl && builtins.substring (nl - sl) sl n == \"{suffix}\") \
       (builtins.attrNames xs)"
    ),
    (None, Some(name)) => format!("xs: xs ? \"{name}\""),
    (None, None) => {
      return Verdict::Error {
        detail: "flake-output-present needs one of suffix/attr".to_string(),
      }
    }
  };
  // Every argument is long-form; the applied function prints a bare boolean.
  let evaluated = match Command::new("nix")
    .args([
      "eval",
      "--extra-experimental-features",
      "nix-command flakes",
      &format!("{}#{attr}", dir.display()),
      "--apply",
      &apply,
    ])
    .output()
  {
    Ok(evaluated) => evaluated,
    Err(error) => {
      return Verdict::Error {
        detail: format!("could not run nix eval for {attr}: {error}"),
      }
    }
  };
  if !evaluated.status.success() {
    return Verdict::Fail {
      detail: format!(
        "{attr} did not evaluate (output absent or flake broken): {}",
        String::from_utf8_lossy(&evaluated.stderr).trim()
      ),
    };
  }
  let want = match (suffix, name) {
    (Some(suffix), _) => format!("attribute ending \"{suffix}\""),
    (None, Some(name)) => format!("attribute \"{name}\""),
    (None, None) => "required attribute".to_string(),
  };
  let printed = String::from_utf8_lossy(&evaluated.stdout);
  match printed.trim() {
    "true" => Verdict::Pass,
    "false" => Verdict::Fail {
      detail: format!("{attr} exposes no {want}"),
    },
    other => Verdict::Error {
      detail: format!("{attr} check returned unexpected \"{other}\""),
    },
  }
}

/// Run one of the template's compliance helper expressions (a file under
/// `nix/compliance/` taking an attrset of string arguments) and return the
/// string it printed.  `subject` names whatever the caller is asserting about,
/// for the diagnostics.
///
/// Some facts about a spawn can only be reached by evaluating Nix — an option
/// default means running the module system, the spawn's own nixpkgs and all.
/// Rather than build such an expression as Rust string concatenation, the
/// expression lives in a reviewable `.nix` file that can be run by hand from
/// the shell, and this function is only the plumbing.
///
/// nix-instantiate rather than `nix eval`: only the former applies `--argstr`
/// to a file that evaluates to a function — `nix eval --file` hands back the
/// uncalled lambda.  Passing parameters as arguments rather than splicing them
/// into an expression is also what frees callers from the bare-identifier rule
/// the `nix eval` kinds need (see `require_flake_ident`); there is no
/// expression for a value to escape into.
///
/// A helper that cannot be run at all is an engine `Error`.  One that exits
/// non-zero, or prints something other than the JSON string it promises, is a
/// `Fail` against `subject` — the spawn's flake is what usually breaks there.
fn nix_helper(
  template_dir: &Path,
  helper: &str,
  subject: &str,
  args: &[(&str, &OsStr)],
) -> Result<String, Verdict> {
  let evaluated = Command::new("nix-instantiate")
    .args([
      "--eval",
      "--strict",
      "--json",
      "--extra-experimental-features",
      "nix-command flakes",
    ])
    .arg(template_dir.join("nix/compliance").join(helper))
    .args(args.iter().flat_map(|(name, value)| {
      [OsStr::new("--argstr"), OsStr::new(name), *value]
    }))
    .output()
    .map_err(|error| Verdict::Error {
      detail: format!("could not run nix-instantiate for {subject}: {error}"),
    })?;
  if !evaluated.status.success() {
    return Err(Verdict::Fail {
      detail: format!(
        "{subject} did not evaluate: {}",
        String::from_utf8_lossy(&evaluated.stderr).trim()
      ),
    });
  }
  // The helper prints a JSON string, so parsing it back is what strips the
  // quoting and any escapes its reason text picked up.
  let printed = String::from_utf8_lossy(&evaluated.stdout);
  serde_json::from_str::<String>(printed.trim()).map_err(|error| {
    Verdict::Error {
      detail: format!(
        "{subject} helper returned unreadable output \"{}\": {error}",
        printed.trim()
      ),
    }
  })
}

/// Evaluate the spawn's module at `module` and compare the default of the
/// option at `option` against `expected`.  `module-option-default.nix` does
/// the reaching-into-the-module part and prints `ok`, `skip: …`, or `fail: …`;
/// this only classifies that.
fn nix_module_option_default(
  dir: &Path,
  template_dir: &Path,
  module: &str,
  option: &str,
  expected: &str,
) -> Verdict {
  nix_helper(
    template_dir,
    "module-option-default.nix",
    module,
    &[
      ("spawn", dir.as_os_str()),
      ("module", OsStr::new(module)),
      ("option", OsStr::new(option)),
      ("expected", OsStr::new(expected)),
    ],
  )
  .map_or_else(
    |verdict| verdict,
    |reported| module_option_verdict(module, &reported),
  )
}

/// Classify what `module-option-default.nix` printed.  Split out from the
/// subprocess so the mapping is testable on its own.
fn module_option_verdict(module: &str, reported: &str) -> Verdict {
  match reported.split_once(": ") {
    None if reported == "ok" => Verdict::Pass,
    Some(("skip", reason)) => Verdict::Skip {
      reason: reason.to_string(),
    },
    Some(("fail", detail)) => Verdict::Fail {
      detail: detail.to_string(),
    },
    _ => Verdict::Error {
      detail: format!("{module} check returned unexpected \"{reported}\""),
    },
  }
}

/// The current host's Nix system double (e.g. `aarch64-darwin`), assembled from
/// the compile target so no subprocess is needed.  Nix spells macOS `darwin`.
fn nix_system() -> String {
  let os = match std::env::consts::OS {
    "macos" => "darwin",
    other => other,
  };
  format!("{}-{os}", std::env::consts::ARCH)
}

// ── Shared helpers ───────────────────────────────────────────────────

/// The outcome of reading a file: present, legitimately absent, or a real I/O
/// error.  Keeping "absent" distinct from "error" lets each check decide
/// whether a missing file is a failure (a required file) or a skip (an
/// optional lockfile).
enum FileRead {
  Found(String),
  Missing,
  Error(String),
}

fn read_file(path: &Path) -> FileRead {
  match std::fs::read_to_string(path) {
    Ok(text) => FileRead::Found(text),
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
      FileRead::Missing
    }
    Err(error) => {
      FileRead::Error(format!("could not read {}: {error}", path.display()))
    }
  }
}

/// The two foundation pins, or a reason they could not be compared.
enum Pins {
  Both { cargo: String, flake: String },
  Skip(String),
  Error(String),
}

fn pins(dir: &Path) -> Pins {
  let cargo = match read_file(&dir.join("Cargo.lock")) {
    FileRead::Found(text) => match pins::cargo_foundation_rev(&text) {
      Ok(Some(rev)) => rev,
      Ok(None) => {
        return Pins::Skip(
          "foundation is not a git dependency in Cargo.lock".to_string(),
        )
      }
      Err(detail) => return Pins::Error(detail),
    },
    FileRead::Missing => {
      return Pins::Skip("Cargo.lock not present".to_string())
    }
    FileRead::Error(detail) => return Pins::Error(detail),
  };
  let flake = match read_file(&dir.join("flake.lock")) {
    FileRead::Found(text) => match pins::flake_foundation_rev(&text) {
      Ok(Some(rev)) => rev,
      Ok(None) => {
        return Pins::Skip("no foundation input in flake.lock".to_string())
      }
      Err(detail) => return Pins::Error(detail),
    },
    FileRead::Missing => {
      return Pins::Skip("flake.lock not present".to_string())
    }
    FileRead::Error(detail) => return Pins::Error(detail),
  };
  Pins::Both { cargo, flake }
}

/// Whether some `crates/**/Cargo.toml` enables the named foundation feature.
/// This mirrors the loose grep the legacy shell check used.
fn foundation_feature_present(dir: &Path, feature: &str) -> bool {
  let mut files = Vec::new();
  walk_files(&dir.join("crates"), &mut files);
  let needle = format!("\"{feature}\"");
  files
    .into_iter()
    .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
    .filter_map(|path| match read_file(&path) {
      FileRead::Found(text) => Some(text),
      _ => None,
    })
    .any(|text| {
      text
        .lines()
        .any(|line| line.contains("features") && line.contains(&needle))
    })
}

/// Source-like files that still mention the bare `rust-template` literal,
/// excluding the expected foundation references and the GitHub URL.  Lockfiles
/// are excluded by extension (`.lock` is not in the scanned set); the
/// provenance file is excluded by name.
fn stale_literals(dir: &Path) -> Vec<String> {
  const SCANNED: [&str; 6] = ["rs", "toml", "nix", "yml", "yaml", "json"];
  let mut files = Vec::new();
  walk_files(dir, &mut files);
  let mut offenders = Vec::new();
  for path in files {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !SCANNED.contains(&extension) {
      continue;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name == "rust-template.json" {
      continue;
    }
    let FileRead::Found(text) = read_file(&path) else {
      continue;
    };
    let stale = text.lines().any(|line| {
      line.contains("rust-template")
        && !line.contains("rust-template-foundation")
        // Any `<owner>/rust-template` reference — the foundation git URL or
        // flake input under any fork's owner — and a `file://` path to an
        // on-disk rust-template both spell "/rust-template"; neither is a stale
        // substitution.  See tasks.org for full fork support.
        && !line.contains("/rust-template")
        // The `rust-template.json` manifest keeps its literal filename in every
        // spawn (the reusable release workflow and the emitted flake.nix both
        // read it by name), so a reference to it — with or without a leading
        // path — is not a stale substitution.
        && !line.contains("rust-template.json")
    });
    if stale {
      offenders.push(relative_display(dir, &path));
    }
  }
  offenders
}

/// Recursively collect files under `root`, skipping VCS and build directories.
/// Best-effort: unreadable directories and entries are logged and skipped
/// rather than aborting the walk.
fn walk_files(root: &Path, out: &mut Vec<PathBuf>) {
  let entries = match std::fs::read_dir(root) {
    Ok(entries) => entries,
    Err(error) => {
      tracing::debug!(
        "skipping unreadable directory {}: {error}",
        root.display()
      );
      return;
    }
  };
  for entry in entries {
    let entry = match entry {
      Ok(entry) => entry,
      Err(error) => {
        tracing::debug!(
          "skipping unreadable entry in {}: {error}",
          root.display()
        );
        continue;
      }
    };
    let path = entry.path();
    if path.is_dir() {
      let name = entry.file_name();
      if name == ".git" || name == "target" || name == ".direnv" {
        continue;
      }
      walk_files(&path, out);
    } else {
      out.push(path);
    }
  }
}

fn relative_display(base: &Path, path: &Path) -> String {
  path
    .strip_prefix(base)
    .unwrap_or(path)
    .display()
    .to_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn glob_matches_per_segment_wildcards() {
    assert!(glob_matches(
      "crates/*/tests/*.rs",
      "crates/cli/tests/integration_test.rs"
    ));
    assert!(!glob_matches("crates/*/tests/*.rs", "crates/cli/src/main.rs"));
    // Segment counts must match.
    assert!(!glob_matches("crates/*/tests/*.rs", "crates/cli/tests"));
    assert!(glob_matches("*.toml", "Cargo.toml"));
    assert!(!glob_matches("*.toml", "Cargo.lock"));
  }

  #[test]
  fn navigate_descends_objects_and_arrays() {
    let value = parse_json(r#"{ "a": { "b": ["x", "y"] } }"#).unwrap();
    assert_eq!(
      navigate(&value, "a.b.1").and_then(scalar_to_string),
      Some("y".to_string())
    );
    assert!(navigate(&value, "a.c").is_none());
    assert!(navigate(&value, "a.b.9").is_none());
  }

  #[test]
  fn parse_toml_normalises_inline_tables() {
    // `version.workspace = true` parses to package.version.workspace == true.
    let value = parse_toml("[package]\nversion.workspace = true\n").unwrap();
    assert_eq!(
      navigate(&value, "package.version.workspace").and_then(scalar_to_string),
      Some("true".to_string())
    );
  }

  #[test]
  fn toml_array_resolves_to_a_sequence() {
    // A treefmt-style includes array resolves to `seq`, so `SeqContains`
    // can assert a given file glob is configured for a formatter.
    let value =
      parse_toml("[formatter.prettier]\nincludes = [\"*.css\", \"*.json\"]\n")
        .unwrap();
    let resolved = resolve_json(&value, "formatter.prettier.includes");
    assert_eq!(
      resolved.seq,
      Some(vec!["*.css".to_string(), "*.json".to_string()])
    );
    assert!(matches!(
      verdict(
        "treefmt.toml",
        "formatter.prettier.includes",
        &PathMatch::SeqContains("*.json"),
        &resolved,
      ),
      Verdict::Pass
    ));
  }

  #[test]
  fn yaml_navigate_descends_and_resolves() {
    let docs = yaml_rust2::YamlLoader::load_from_str(
      "on:\n  push:\n    branches: [main]\njobs:\n  ci:\n    uses: org/repo/x.yml@main\n",
    )
    .unwrap();
    let doc = &docs[0];
    // The `on:` trigger key resolves despite the YAML-1.1 boolean quirk.
    assert_eq!(
      resolve_yaml(doc, "on.push.branches").seq,
      Some(vec!["main".to_string()])
    );
    assert_eq!(
      resolve_yaml(doc, "jobs.ci.uses").scalar.as_deref(),
      Some("org/repo/x.yml@main")
    );
    assert!(!resolve_yaml(doc, "jobs.missing").exists);
  }

  #[test]
  fn not_contains_forbids_only_the_present_needle() {
    // A wrapper still carrying the superseded guard fails; a spawn's own
    // unrelated guard passes, and an absent guard passes.
    let needle = "github.event.workflow_run.conclusion == 'success'";
    let holding = Resolved {
      exists: true,
      scalar: Some(needle.to_string()),
      seq: None,
    };
    let custom = Resolved {
      exists: true,
      scalar: Some("github.repository == 'owner/fork'".to_string()),
      seq: None,
    };
    assert!(matches!(
      verdict(
        "w.yml",
        "jobs.automerge.if",
        &PathMatch::NotContains(needle),
        &holding,
      ),
      Verdict::Fail { .. }
    ));
    assert!(matches!(
      verdict(
        "w.yml",
        "jobs.automerge.if",
        &PathMatch::NotContains(needle),
        &custom,
      ),
      Verdict::Pass
    ));
    assert!(matches!(
      verdict(
        "w.yml",
        "jobs.automerge.if",
        &PathMatch::NotContains(needle),
        &Resolved::absent(),
      ),
      Verdict::Pass
    ));
  }

  #[test]
  fn module_option_verdict_maps_each_helper_outcome() {
    let module = "darwinModules.server";
    assert!(matches!(module_option_verdict(module, "ok"), Verdict::Pass));
    // A spawn with no such module output is not drift — it has nothing to
    // check — so it skips rather than failing the fleet.
    assert!(matches!(
      module_option_verdict(
        module,
        "skip: flake exposes no darwinModules.server"
      ),
      Verdict::Skip { .. }
    ));
    // The helper's reason carries through as the failure detail, so the report
    // names the actual default rather than just "mismatch".
    assert!(matches!(
      module_option_verdict(
        module,
        "fail: services.app-server.logPathStdout defaults to \"/x\"",
      ),
      Verdict::Fail { detail } if detail.starts_with("services.app-server"),
    ));
    // Anything the helper is not documented to print is an engine problem, not
    // a verdict about the spawn.
    assert!(matches!(
      module_option_verdict(module, "who knows"),
      Verdict::Error { .. }
    ));
  }

  #[test]
  fn justfile_recipe_queries_just_not_the_text() {
    // Skip when `just` is absent (e.g. `cargo test` outside the dev shell); in
    // CI it is present, so the assertions run there.
    if Command::new("just").arg("--version").output().is_err() {
      return;
    }
    let dir =
      std::env::temp_dir().join(format!("cdb-just-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
      dir.join("justfile"),
      "# ghost is only named in this comment\nreal *args:\n    echo {{args}}\n",
    )
    .unwrap();
    // A real recipe passes; a name that appears only in a comment does not —
    // the whole point of querying `just --summary` over a text search.
    assert!(matches!(justfile_recipe(&dir, "real"), Verdict::Pass));
    assert!(matches!(justfile_recipe(&dir, "ghost"), Verdict::Fail { .. }));
    assert!(matches!(
      justfile_recipe(&dir.join("missing"), "real"),
      Verdict::Skip { .. }
    ));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn rust_ast_predicates() {
    let source = r#"
      use rust_template_foundation::logging::*;
      #[derive(Debug, MergeConfig)]
      #[merge_config(app_name = "x")]
      struct Config {
        #[merge_config(common)]
        a: u8,
        #[merge_config(common)]
        b: u8,
        #[merge_config(short)]
        c: u8,
      }
      #[foundation_main]
      fn main() {}
      impl ServerApp for Config {}
      fn run() {
        thing.with_state(state).listen();
      }
    "#;
    let file = syn::parse_file(source).unwrap();

    let main = find_fn(&file, "main").unwrap();
    assert!(main
      .attrs
      .iter()
      .any(|a| attr_last_segment(a).as_deref() == Some("foundation_main")));

    let config = find_struct(&file, "Config").unwrap();
    assert!(struct_has_derive(config, "MergeConfig"));
    assert!(config
      .attrs
      .iter()
      .any(|a| attr_last_segment(a).as_deref() == Some("merge_config")));

    let commons = config
      .fields
      .iter()
      .filter(|field| {
        field.attrs.iter().any(|a| {
          attr_last_segment(a).as_deref() == Some("merge_config")
            && attr_tokens_contain(a, "common")
        })
      })
      .count();
    assert_eq!(commons, 2);

    assert!(file
      .items
      .iter()
      .any(|item| use_glob_matches(item, "rust_template_foundation::logging")));
    assert!(file.items.iter().any(|item| impl_matches(
      item,
      "ServerApp",
      "Config"
    )));

    let run = find_fn(&file, "run").unwrap();
    let mut visitor = MethodVisitor::default();
    syn::visit::visit_item_fn(&mut visitor, run);
    assert!(visitor.methods.contains("with_state"));
    assert!(visitor.methods.contains("listen"));
  }
}
