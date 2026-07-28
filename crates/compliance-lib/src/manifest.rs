//! The declarative check manifest — `compliance-checks.toml`.
//!
//! Each `[[check]]` entry names a stable `id`, a human `description`, a `kind`
//! from the finite set the engine implements, and the parameters that kind
//! needs.  An optional `when_crates_contains` restricts a check to spawns whose
//! crate-role list contains a given substring (e.g. only run a server check on
//! spawns generated with a `server` crate); an optional
//! `when_foundation_feature` restricts it to spawns that enable a given
//! foundation feature (e.g. only run the Apple-SDK wiring check on spawns that
//! enable `auth`).
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
  /// When set, the check only applies to spawns that enable this foundation
  /// feature (any `crates/**/Cargo.toml` lists it on the foundation
  /// dependency); others report `Skip`.  Gates feature-coupled wiring — e.g.
  /// an auth spawn that must wire the Apple SDK its TLS stack links.
  pub when_foundation_feature: Option<String>,
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
  /// `target` (an org document) contains the heading reached by the `section`
  /// path: a sequence of nested heading titles, outermost first.  A top-level
  /// heading is a one-element path; a longer path enforces nesting.
  SectionExists {
    target: String,
    section: Vec<String>,
  },
  /// `target` contains `contains` as a substring after line-wrapped
  /// paragraphs are flattened; when `section` is set, the search is scoped to
  /// the body of the section at that path.
  MentionPresent {
    target: String,
    section: Option<Vec<String>>,
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
  /// `target` parses as TOML and the sequence at `pointer` contains an element
  /// equal to `value`.
  TomlSeqContains {
    target: String,
    pointer: String,
    value: String,
  },
  /// Every `crates/*/Cargo.toml` parses as TOML and its scalar at `pointer`
  /// equals `value`.  There is no `target`: the crate manifests are discovered,
  /// not named, so the check covers archetype and custom-named crates alike.
  CrateTomlPathEquals { pointer: String, value: String },
  /// `target` parses as YAML and the value at `pointer` exists.
  YamlPathExists { target: String, pointer: String },
  /// `target` parses as YAML and the scalar at `pointer` equals `value`.
  YamlPathEquals {
    target: String,
    pointer: String,
    value: String,
  },
  /// `target` parses as YAML and the scalar at `pointer` contains `contains`
  /// as a substring (for folded `if:` guards and the like).
  YamlPathContains {
    target: String,
    pointer: String,
    contains: String,
  },
  /// `target` parses as YAML and the scalar at `pointer` does NOT contain
  /// `contains` as a substring — and an absent pointer passes.  Forbids a
  /// specific superseded value (e.g. an inline guard a reusable workflow now
  /// owns) without forbidding an unrelated value a spawn set for its own
  /// reasons, and without requiring the pointer to be present at all.
  YamlPathNotContains {
    target: String,
    pointer: String,
    contains: String,
  },
  /// The spawn's `just --summary` lists a recipe named `recipe`.  Unlike a text
  /// search this queries just's own parser, so a mention in a comment does not
  /// satisfy it — only a real recipe definition does.
  JustfileRecipe { recipe: String },
  /// `target` parses as YAML and the sequence at `pointer` contains a scalar
  /// equal to `value`.
  YamlSeqContains {
    target: String,
    pointer: String,
    value: String,
  },
  /// `target` parses as Rust and the fn named `function` carries an attribute
  /// whose path's last segment is `attr` (any local alias matches).
  RustFnHasAttr {
    target: String,
    function: String,
    attr: String,
  },
  /// The struct named `struct_name` has a `#[derive(...)]` that lists `derive`.
  RustStructHasDerive {
    target: String,
    struct_name: String,
    derive: String,
  },
  /// The struct named `struct_name` carries a helper attribute whose path's
  /// last segment is `attr` (e.g. `#[merge_config(...)]`).
  RustStructHasHelperAttr {
    target: String,
    struct_name: String,
    attr: String,
  },
  /// Exactly `count` fields of `struct_name` carry a helper attribute whose
  /// path's last segment is `attr`; when `contains` is set, the attribute's
  /// token text must also contain it (e.g. the nested `common` marker).
  RustStructFieldAttrCount {
    target: String,
    struct_name: String,
    attr: String,
    contains: Option<String>,
    count: u32,
  },
  /// `target` contains a glob `use` import `use <path>::*;`.
  RustUseGlob { target: String, path: String },
  /// `target` has an `impl <trait_name> for <self_ty>`.
  RustImplTraitFor {
    target: String,
    trait_name: String,
    self_ty: String,
  },
  /// Within the fn named `function`, a method-call chain invokes every method
  /// ident listed in `methods`.
  RustMethodChain {
    target: String,
    function: String,
    methods: Vec<String>,
  },
  /// `nix eval` of `devShells.<system>.<shell>.<var>` (the flake's default
  /// devShell when `shell` is `None`) equals `value`.  Unlike the file-reading
  /// kinds this evaluates the flake, so it proves the shell resolves and
  /// declares the variable — mkShell turns the attribute into the environment
  /// variable the shell exports, so the evaluated value is what the shell would
  /// print at runtime.  Requires the spawn's foundation input to resolve, so
  /// tests run it against a spawn localized to the template under test.
  DevShellEnv {
    shell: Option<String>,
    var: String,
    value: String,
  },
  /// `nix eval` of the spawn's devShell (the flake's default devShell when
  /// `shell` is `None`) has a build input whose derivation name is `package`.
  /// Reading the resolved shell's inputs rather than grepping flake.nix means
  /// only a package actually pulled into the shell satisfies it — a mention in
  /// a comment or an unrelated string does not.  Like DevShellEnv this
  /// evaluates the flake (proving the shell resolves) without realizing the
  /// shell's closure, and requires the spawn's foundation input to resolve, so
  /// tests run it against a spawn localized to the template under test.
  DevShellPackage {
    shell: Option<String>,
    package: String,
  },
  /// `nix eval` the flake output attrset `<output>.<system>` (the host
  /// double when `system` is `None`) and confirm it exposes a required
  /// attribute: with `suffix` set, any attribute whose name ends with it (a
  /// per-crate package such as `<crate>-x86_64-windows`, whose prefix varies
  /// per spawn); with `name` set, that exact attribute.  Exactly one of the
  /// two is set.  Unlike the file-reading kinds this evaluates the flake, so
  /// it proves the output actually resolves rather than that a call producing
  /// it appears in the text.  Requires the spawn's foundation input to
  /// resolve, so tests run it against a spawn localized to the template under
  /// test.
  FlakeOutputPresent {
    output: String,
    system: Option<String>,
    suffix: Option<String>,
    name: Option<String>,
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
  when_foundation_feature: Option<String>,
  #[serde(default)]
  target: Option<String>,
  #[serde(default)]
  path: Option<String>,
  #[serde(default)]
  section: Option<Vec<String>>,
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
  #[serde(default)]
  function: Option<String>,
  #[serde(default)]
  struct_name: Option<String>,
  #[serde(default)]
  derive: Option<String>,
  #[serde(default)]
  attr: Option<String>,
  #[serde(default)]
  count: Option<u32>,
  #[serde(default)]
  trait_name: Option<String>,
  #[serde(default)]
  self_ty: Option<String>,
  #[serde(default)]
  methods: Option<String>,
  #[serde(default)]
  shell: Option<String>,
  #[serde(default)]
  var: Option<String>,
  #[serde(default)]
  output: Option<String>,
  #[serde(default)]
  system: Option<String>,
  #[serde(default)]
  suffix: Option<String>,
  #[serde(default)]
  recipe: Option<String>,
  #[serde(default)]
  package: Option<String>,
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

/// Require an integer parameter.
fn require_u32(
  id: &str,
  kind: &str,
  name: &str,
  value: Option<u32>,
) -> Result<u32, ComplianceError> {
  value.ok_or_else(|| ComplianceError::ManifestInvalid {
    id: id.to_string(),
    message: format!("kind '{kind}' requires integer parameter '{name}'"),
  })
}

/// Require a non-empty section-path parameter (a list of heading titles).
fn require_path(
  id: &str,
  kind: &str,
  name: &str,
  value: Option<Vec<String>>,
) -> Result<Vec<String>, ComplianceError> {
  match value {
    Some(path) if !path.is_empty() => Ok(path),
    _ => Err(ComplianceError::ManifestInvalid {
      id: id.to_string(),
      message: format!("kind '{kind}' requires a non-empty list '{name}'"),
    }),
  }
}

/// Confirm a flake attribute name (or suffix) is a bare identifier —
/// alphanumerics plus `.`, `_`, `-`.  It is interpolated verbatim into the
/// `nix eval --apply` expression, so restricting it to these characters keeps
/// that expression injection-free without any quoting gymnastics.
fn require_flake_ident(
  id: &str,
  kind: &str,
  name: &str,
  value: &str,
) -> Result<(), ComplianceError> {
  let ok = !value.is_empty()
    && value
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
  if ok {
    Ok(())
  } else {
    Err(ComplianceError::ManifestInvalid {
      id: id.to_string(),
      message: format!(
        "kind '{kind}' parameter '{name}' must be a bare flake attribute name \
         (letters, digits, '.', '_', '-')"
      ),
    })
  }
}

/// Reject an empty section path while leaving an absent one as `None`.
fn optional_path(
  id: &str,
  kind: &str,
  name: &str,
  value: Option<Vec<String>>,
) -> Result<Option<Vec<String>>, ComplianceError> {
  match value {
    Some(path) if path.is_empty() => Err(ComplianceError::ManifestInvalid {
      id: id.to_string(),
      message: format!("kind '{kind}' has an empty list '{name}'"),
    }),
    other => Ok(other),
  }
}

impl RawCheck {
  fn validate(self) -> Result<Check, ComplianceError> {
    let RawCheck {
      id,
      description,
      kind,
      when_crates_contains,
      when_public,
      when_foundation_feature,
      target,
      path,
      section,
      contains,
      feature,
      glob,
      pointer,
      value,
      function,
      struct_name,
      derive,
      attr,
      count,
      trait_name,
      self_ty,
      methods,
      shell,
      var,
      output,
      system,
      suffix,
      recipe,
      package,
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
        section: require_path(&id, &kind, "section", section)?,
      },
      "mention-present" => CheckKind::MentionPresent {
        target: require(&id, &kind, "target", target)?,
        section: optional_path(&id, &kind, "section", section)?,
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
      "toml-seq-contains" => CheckKind::TomlSeqContains {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
        value: require(&id, &kind, "value", value)?,
      },
      "crate-toml-path-equals" => CheckKind::CrateTomlPathEquals {
        pointer: require(&id, &kind, "pointer", pointer)?,
        value: require(&id, &kind, "value", value)?,
      },
      "yaml-path-exists" => CheckKind::YamlPathExists {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
      },
      "yaml-path-equals" => CheckKind::YamlPathEquals {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
        value: require(&id, &kind, "value", value)?,
      },
      "yaml-path-contains" => CheckKind::YamlPathContains {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
        contains: require(&id, &kind, "contains", contains)?,
      },
      "yaml-path-not-contains" => CheckKind::YamlPathNotContains {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
        contains: require(&id, &kind, "contains", contains)?,
      },
      "justfile-recipe" => CheckKind::JustfileRecipe {
        recipe: require(&id, &kind, "recipe", recipe)?,
      },
      "yaml-seq-contains" => CheckKind::YamlSeqContains {
        target: require(&id, &kind, "target", target)?,
        pointer: require(&id, &kind, "pointer", pointer)?,
        value: require(&id, &kind, "value", value)?,
      },
      "rust-fn-has-attr" => CheckKind::RustFnHasAttr {
        target: require(&id, &kind, "target", target)?,
        function: require(&id, &kind, "function", function)?,
        attr: require(&id, &kind, "attr", attr)?,
      },
      "rust-struct-has-derive" => CheckKind::RustStructHasDerive {
        target: require(&id, &kind, "target", target)?,
        struct_name: require(&id, &kind, "struct_name", struct_name)?,
        derive: require(&id, &kind, "derive", derive)?,
      },
      "rust-struct-has-helper-attr" => CheckKind::RustStructHasHelperAttr {
        target: require(&id, &kind, "target", target)?,
        struct_name: require(&id, &kind, "struct_name", struct_name)?,
        attr: require(&id, &kind, "attr", attr)?,
      },
      "rust-struct-field-attr-count" => CheckKind::RustStructFieldAttrCount {
        target: require(&id, &kind, "target", target)?,
        struct_name: require(&id, &kind, "struct_name", struct_name)?,
        attr: require(&id, &kind, "attr", attr)?,
        contains,
        count: require_u32(&id, &kind, "count", count)?,
      },
      "rust-use-glob" => CheckKind::RustUseGlob {
        target: require(&id, &kind, "target", target)?,
        path: require(&id, &kind, "path", path)?,
      },
      "rust-impl-trait-for" => CheckKind::RustImplTraitFor {
        target: require(&id, &kind, "target", target)?,
        trait_name: require(&id, &kind, "trait_name", trait_name)?,
        self_ty: require(&id, &kind, "self_ty", self_ty)?,
      },
      "rust-method-chain" => CheckKind::RustMethodChain {
        target: require(&id, &kind, "target", target)?,
        function: require(&id, &kind, "function", function)?,
        methods: require(&id, &kind, "methods", methods)?
          .split(',')
          .map(|method| method.trim().to_string())
          .filter(|method| !method.is_empty())
          .collect(),
      },
      "dev-shell-env" => CheckKind::DevShellEnv {
        shell,
        var: require(&id, &kind, "var", var)?,
        value: require(&id, &kind, "value", value)?,
      },
      "dev-shell-package" => {
        let package = require(&id, &kind, "package", package)?;
        // `package` is interpolated into the nix-eval predicate, so hold it to
        // the bare-identifier rule that keeps the whole expression
        // injection-free.
        require_flake_ident(&id, &kind, "package", &package)?;
        CheckKind::DevShellPackage { shell, package }
      }
      // `attr` doubles as the exact-name selector here; `suffix` selects by
      // name suffix.  Exactly one must be set — a suffix for the per-crate
      // package outputs, an exact name for a fixed output like a flake check.
      "flake-output-present" => {
        let output = require(&id, &kind, "output", output)?;
        // `output` and `system` are interpolated into the flake reference the
        // same way the selectors are, so hold them to the same bare-identifier
        // rule to keep the whole `nix eval` expression injection-free.
        require_flake_ident(&id, &kind, "output", &output)?;
        if let Some(system) = system.as_deref() {
          require_flake_ident(&id, &kind, "system", system)?;
        }
        match (suffix, attr) {
          (Some(suffix), None) => {
            require_flake_ident(&id, &kind, "suffix", &suffix)?;
            CheckKind::FlakeOutputPresent {
              output,
              system,
              suffix: Some(suffix),
              name: None,
            }
          }
          (None, Some(name)) => {
            require_flake_ident(&id, &kind, "attr", &name)?;
            CheckKind::FlakeOutputPresent {
              output,
              system,
              suffix: None,
              name: Some(name),
            }
          }
          _ => {
            return Err(ComplianceError::ManifestInvalid {
              id: id.clone(),
              message: format!(
                "kind '{kind}' requires exactly one of 'suffix' or 'attr'"
              ),
            })
          }
        }
      }
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
      when_foundation_feature,
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
            target = "CHANGELOG.org"
            section = ["Upcoming", "Added"]
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let check = raw.check.into_iter().next().unwrap().validate().unwrap();
    assert_eq!(check.id, "x");
    match check.kind {
      CheckKind::SectionExists { target, section } => {
        assert_eq!(target, "CHANGELOG.org");
        assert_eq!(section, vec!["Upcoming", "Added"]);
      }
      other => panic!("wrong kind: {other:?}"),
    }
  }

  #[test]
  fn validates_a_dev_shell_env_check() {
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "dev-shell-env"
            shell = "ci"
            var = "RUST_TEMPLATE_SHELL"
            value = "ci"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let check = raw.check.into_iter().next().unwrap().validate().unwrap();
    match check.kind {
      CheckKind::DevShellEnv { shell, var, value } => {
        assert_eq!(shell.as_deref(), Some("ci"));
        assert_eq!(var, "RUST_TEMPLATE_SHELL");
        assert_eq!(value, "ci");
      }
      other => panic!("wrong kind: {other:?}"),
    }
  }

  #[test]
  fn validates_a_flake_output_present_suffix_check() {
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "flake-output-present"
            output = "packages"
            suffix = "-x86_64-windows"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let check = raw.check.into_iter().next().unwrap().validate().unwrap();
    match check.kind {
      CheckKind::FlakeOutputPresent {
        output,
        system,
        suffix,
        name,
      } => {
        assert_eq!(output, "packages");
        assert!(system.is_none());
        assert_eq!(suffix.as_deref(), Some("-x86_64-windows"));
        assert!(name.is_none());
      }
      other => panic!("wrong kind: {other:?}"),
    }
  }

  #[test]
  fn validates_a_flake_output_present_name_check() {
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "flake-output-present"
            output = "checks"
            system = "x86_64-linux"
            attr = "windowsSmoke"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let check = raw.check.into_iter().next().unwrap().validate().unwrap();
    match check.kind {
      CheckKind::FlakeOutputPresent {
        output,
        system,
        suffix,
        name,
      } => {
        assert_eq!(output, "checks");
        assert_eq!(system.as_deref(), Some("x86_64-linux"));
        assert!(suffix.is_none());
        assert_eq!(name.as_deref(), Some("windowsSmoke"));
      }
      other => panic!("wrong kind: {other:?}"),
    }
  }

  #[test]
  fn flake_output_present_needs_exactly_one_selector() {
    // Neither suffix nor attr set: ambiguous, so validation must reject it.
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "flake-output-present"
            output = "packages"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let result = raw.check.into_iter().next().unwrap().validate();
    assert!(result.is_err());
  }

  #[test]
  fn dev_shell_env_shell_defaults_to_none() {
    // An omitted `shell` targets the flake's default devShell.
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "dev-shell-env"
            var = "RUST_TEMPLATE_SHELL"
            value = "default"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let check = raw.check.into_iter().next().unwrap().validate().unwrap();
    match check.kind {
      CheckKind::DevShellEnv { shell, .. } => assert!(shell.is_none()),
      other => panic!("wrong kind: {other:?}"),
    }
  }

  #[test]
  fn empty_section_path_is_an_error() {
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "section-exists"
            target = "llms.org"
            section = []
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
  fn carries_when_foundation_feature() {
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "json-path-equals"
            target = "rust-template.json"
            pointer = "apple-frameworks"
            value = "true"
            when_foundation_feature = "auth"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let check = raw.check.into_iter().next().unwrap().validate().unwrap();
    assert_eq!(check.when_foundation_feature.as_deref(), Some("auth"));
  }

  #[test]
  fn when_foundation_feature_defaults_to_none() {
    let toml = r#"
            [[check]]
            id = "x"
            description = "d"
            kind = "file-present"
            path = "flake.nix"
        "#;
    let raw: RawManifest = toml::from_str(toml).unwrap();
    let check = raw.check.into_iter().next().unwrap().validate().unwrap();
    assert!(check.when_foundation_feature.is_none());
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
