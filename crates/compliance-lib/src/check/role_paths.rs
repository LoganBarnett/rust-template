//! Role path resolution: a check written against the archetype layout lands on
//! the crate that plays the role.

use crate::manifest::CheckKind;
use crate::provenance::Provenance;

/// Rewrite every spawn file path in `kind` through the spawn's role
/// declarations.  A path such as `crates/cli/src/config.rs` moves to the
/// directory of the crate declared as `cli`.  The match is exhaustive on
/// purpose.  A new kind cannot compile until it decides here whether its
/// parameters are spawn file paths.
pub(super) fn resolve_kind(
  kind: &CheckKind,
  provenance: &Provenance,
) -> CheckKind {
  let resolve = |path: &str| provenance.resolve_role_path(path);
  match kind {
    CheckKind::FilePresent { path } => CheckKind::FilePresent {
      path: resolve(path),
    },
    CheckKind::SymlinkPointsTo { path, points_to } => {
      CheckKind::SymlinkPointsTo {
        path: resolve(path),
        points_to: resolve(points_to),
      }
    }
    CheckKind::JsonValid { path } => CheckKind::JsonValid {
      path: resolve(path),
    },
    CheckKind::FileContains { target, contains } => CheckKind::FileContains {
      target: resolve(target),
      contains: contains.clone(),
    },
    CheckKind::SectionExists { target, section } => CheckKind::SectionExists {
      target: resolve(target),
      section: section.clone(),
    },
    CheckKind::MentionPresent {
      target,
      section,
      contains,
    } => CheckKind::MentionPresent {
      target: resolve(target),
      section: section.clone(),
      contains: contains.clone(),
    },
    CheckKind::LinePresent { target, line } => CheckKind::LinePresent {
      target: resolve(target),
      line: line.clone(),
    },
    CheckKind::TemplateParagraphsInOrder { target, section } => {
      CheckKind::TemplateParagraphsInOrder {
        target: resolve(target),
        section: section.clone(),
      }
    }
    CheckKind::FileAbsent { path } => CheckKind::FileAbsent {
      path: resolve(path),
    },
    // The spawn side is resolved in its match arm instead, because the same
    // path also names the canonical copy under `template/`, which keeps its
    // archetype layout.
    CheckKind::FileMatchesTemplate { .. } => kind.clone(),
    CheckKind::GlobPresent { glob } => CheckKind::GlobPresent {
      glob: resolve(glob),
    },
    CheckKind::JsonPathExists { target, pointer } => {
      CheckKind::JsonPathExists {
        target: resolve(target),
        pointer: pointer.clone(),
      }
    }
    CheckKind::JsonPathEquals {
      target,
      pointer,
      value,
    } => CheckKind::JsonPathEquals {
      target: resolve(target),
      pointer: pointer.clone(),
      value: value.clone(),
    },
    CheckKind::JsonSeqContains {
      target,
      pointer,
      value,
    } => CheckKind::JsonSeqContains {
      target: resolve(target),
      pointer: pointer.clone(),
      value: value.clone(),
    },
    CheckKind::TomlPathExists { target, pointer } => {
      CheckKind::TomlPathExists {
        target: resolve(target),
        pointer: pointer.clone(),
      }
    }
    CheckKind::TomlPathEquals {
      target,
      pointer,
      value,
    } => CheckKind::TomlPathEquals {
      target: resolve(target),
      pointer: pointer.clone(),
      value: value.clone(),
    },
    CheckKind::TomlSeqContains {
      target,
      pointer,
      value,
    } => CheckKind::TomlSeqContains {
      target: resolve(target),
      pointer: pointer.clone(),
      value: value.clone(),
    },
    CheckKind::YamlPathExists { target, pointer } => {
      CheckKind::YamlPathExists {
        target: resolve(target),
        pointer: pointer.clone(),
      }
    }
    CheckKind::YamlPathEquals {
      target,
      pointer,
      value,
    } => CheckKind::YamlPathEquals {
      target: resolve(target),
      pointer: pointer.clone(),
      value: value.clone(),
    },
    CheckKind::YamlPathContains {
      target,
      pointer,
      contains,
    } => CheckKind::YamlPathContains {
      target: resolve(target),
      pointer: pointer.clone(),
      contains: contains.clone(),
    },
    CheckKind::YamlPathNotContains {
      target,
      pointer,
      contains,
    } => CheckKind::YamlPathNotContains {
      target: resolve(target),
      pointer: pointer.clone(),
      contains: contains.clone(),
    },
    CheckKind::YamlSeqContains {
      target,
      pointer,
      value,
    } => CheckKind::YamlSeqContains {
      target: resolve(target),
      pointer: pointer.clone(),
      value: value.clone(),
    },
    // `suite` is relative to the template, so only `target` resolves.
    CheckKind::TemplateSuitePasses { suite, target } => {
      CheckKind::TemplateSuitePasses {
        suite: suite.clone(),
        target: resolve(target),
      }
    }
    CheckKind::RustFnHasAttr {
      target,
      function,
      attr,
    } => CheckKind::RustFnHasAttr {
      target: resolve(target),
      function: function.clone(),
      attr: attr.clone(),
    },
    CheckKind::RustStructHasDerive {
      target,
      struct_name,
      derive,
    } => CheckKind::RustStructHasDerive {
      target: resolve(target),
      struct_name: struct_name.clone(),
      derive: derive.clone(),
    },
    CheckKind::RustStructHasHelperAttr {
      target,
      struct_name,
      attr,
    } => CheckKind::RustStructHasHelperAttr {
      target: resolve(target),
      struct_name: struct_name.clone(),
      attr: attr.clone(),
    },
    CheckKind::RustStructFieldAttrCount {
      target,
      struct_name,
      attr,
      contains,
      count,
    } => CheckKind::RustStructFieldAttrCount {
      target: resolve(target),
      struct_name: struct_name.clone(),
      attr: attr.clone(),
      contains: contains.clone(),
      count: *count,
    },
    // `path` here is a Rust module path, not a file path.
    CheckKind::RustUseGlob { target, path } => CheckKind::RustUseGlob {
      target: resolve(target),
      path: path.clone(),
    },
    CheckKind::RustImplTraitFor {
      target,
      trait_name,
      self_ty,
    } => CheckKind::RustImplTraitFor {
      target: resolve(target),
      trait_name: trait_name.clone(),
      self_ty: self_ty.clone(),
    },
    CheckKind::RustMethodChain {
      target,
      function,
      methods,
    } => CheckKind::RustMethodChain {
      target: resolve(target),
      function: function.clone(),
      methods: methods.clone(),
    },
    // Without a target the whole tree is searched, which needs no resolving.
    CheckKind::NixCallPresent { function, target } => {
      CheckKind::NixCallPresent {
        function: function.clone(),
        target: target.as_deref().map(resolve),
      }
    }
    // The remaining kinds carry no spawn file paths.  They discover files
    // themselves, evaluate the flake, or take other parameters.
    CheckKind::FoundationFeature { .. }
    | CheckKind::NoStaleLiteral
    | CheckKind::PinsAgree
    | CheckKind::PinsCurrent
    | CheckKind::CrateTomlPathEquals { .. }
    | CheckKind::JustfileRecipe { .. }
    | CheckKind::DevShellEnv { .. }
    | CheckKind::DevShellPackage { .. }
    | CheckKind::DevShellPackageAbsent { .. }
    | CheckKind::FlakeOutputPresent { .. }
    | CheckKind::NixModuleOptionDefault { .. }
    | CheckKind::DarwinModuleEvaluates { .. }
    | CheckKind::NixosModuleEvaluates { .. }
    | CheckKind::ProvenanceSchema => kind.clone(),
  }
}
