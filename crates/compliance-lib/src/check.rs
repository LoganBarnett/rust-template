//! Running a single check against a single spawn.
//!
//! Every check resolves to a [`CheckOutcome`].  Outcomes are data, not control
//! flow: a failing or erroring check never aborts the run, so one broken spawn
//! cannot hide the state of the others.  The outcome variants serialize
//! directly into the JSON report.

use crate::manifest::{Check, CheckKind};
use crate::org;
use crate::pins;
use crate::provenance::Provenance;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The result of running one check.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CheckOutcome {
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

impl CheckOutcome {
  /// Whether this outcome should make the overall run fail.
  pub fn is_failure(&self) -> bool {
    matches!(self, CheckOutcome::Fail { .. } | CheckOutcome::Error { .. })
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
}

/// Run `check` against the spawn described by `ctx`.
pub fn run_check(check: &Check, ctx: &SpawnContext) -> CheckOutcome {
  // An explicit opt-out wins over everything else.
  if let Some(reason) = ctx.provenance.ignored_reason(&check.id) {
    return CheckOutcome::Ignored { reason };
  }

  // Role-conditional checks skip on spawns that lack the role.
  if let Some(role) = &check.when_crates_contains {
    if !ctx.crates.contains(role.as_str()) {
      return CheckOutcome::Skip {
        reason: format!(
          "crate roles \"{}\" do not include \"{role}\"",
          ctx.crates
        ),
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
  }
}

// ── Per-kind implementations ─────────────────────────────────────────

fn file_present(dir: &Path, path: &str) -> CheckOutcome {
  if dir.join(path).exists() {
    CheckOutcome::Pass
  } else {
    CheckOutcome::Fail {
      detail: format!("required file missing: {path}"),
    }
  }
}

fn json_valid(dir: &Path, path: &str) -> CheckOutcome {
  match read_file(&dir.join(path)) {
    FileRead::Found(text) => {
      match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(_) => CheckOutcome::Pass,
        Err(error) => CheckOutcome::Fail {
          detail: format!("{path} is not valid JSON: {error}"),
        },
      }
    }
    FileRead::Missing => CheckOutcome::Fail {
      detail: format!("{path} not found"),
    },
    FileRead::Error(detail) => CheckOutcome::Error { detail },
  }
}

fn file_contains(dir: &Path, target: &str, needle: &str) -> CheckOutcome {
  match read_file(&dir.join(target)) {
    FileRead::Found(text) if text.contains(needle) => CheckOutcome::Pass,
    FileRead::Found(_) => CheckOutcome::Fail {
      detail: format!("{target} does not contain \"{needle}\""),
    },
    FileRead::Missing => CheckOutcome::Fail {
      detail: format!("{target} not found"),
    },
    FileRead::Error(detail) => CheckOutcome::Error { detail },
  }
}

fn foundation_feature(dir: &Path, feature: &str) -> CheckOutcome {
  if foundation_feature_present(dir, feature) {
    CheckOutcome::Pass
  } else {
    CheckOutcome::Fail {
      detail: format!("no crate enables the foundation \"{feature}\" feature"),
    }
  }
}

fn no_stale_literal(dir: &Path) -> CheckOutcome {
  let offenders = stale_literals(dir);
  if offenders.is_empty() {
    CheckOutcome::Pass
  } else {
    CheckOutcome::Fail {
      detail: format!(
        "stale rust-template literals in: {}",
        offenders.join(", ")
      ),
    }
  }
}

fn section_exists(dir: &Path, target: &str, section: &str) -> CheckOutcome {
  match read_file(&dir.join(target)) {
    FileRead::Found(text) if org::section_exists(&text, section) => {
      CheckOutcome::Pass
    }
    FileRead::Found(_) => CheckOutcome::Fail {
      detail: format!("section \"{section}\" not found in {target}"),
    },
    FileRead::Missing => CheckOutcome::Fail {
      detail: format!("{target} not found"),
    },
    FileRead::Error(detail) => CheckOutcome::Error { detail },
  }
}

fn mention_present(
  dir: &Path,
  target: &str,
  section: Option<&str>,
  needle: &str,
) -> CheckOutcome {
  match read_file(&dir.join(target)) {
    FileRead::Found(text) => match org::mention_present(&text, section, needle)
    {
      Ok(true) => CheckOutcome::Pass,
      Ok(false) => CheckOutcome::Fail {
        detail: section.map_or_else(
          || format!("\"{needle}\" not found in {target}"),
          |name| format!("\"{needle}\" not found in {target} § {name}"),
        ),
      },
      Err(reason) => CheckOutcome::Fail {
        detail: format!("{reason} in {target}"),
      },
    },
    FileRead::Missing => CheckOutcome::Fail {
      detail: format!("{target} not found"),
    },
    FileRead::Error(detail) => CheckOutcome::Error { detail },
  }
}

fn pins_agree(dir: &Path) -> CheckOutcome {
  match resolve_pins(dir) {
    Pins::Both { cargo, flake } if cargo == flake => CheckOutcome::Pass,
    Pins::Both { cargo, flake } => CheckOutcome::Fail {
      detail: format!("Cargo.lock pins {cargo}; flake.lock pins {flake}"),
    },
    Pins::Skip(reason) => CheckOutcome::Skip { reason },
    Pins::Error(detail) => CheckOutcome::Error { detail },
  }
}

fn pins_current(
  dir: &Path,
  template_head: &Result<String, String>,
) -> CheckOutcome {
  let (cargo, flake) = match resolve_pins(dir) {
    Pins::Both { cargo, flake } => (cargo, flake),
    Pins::Skip(reason) => return CheckOutcome::Skip { reason },
    Pins::Error(detail) => return CheckOutcome::Error { detail },
  };
  let head = match template_head {
    Ok(head) => head,
    Err(reason) => {
      return CheckOutcome::Error {
        detail: format!("template HEAD unavailable: {reason}"),
      }
    }
  };
  if &cargo == head && &flake == head {
    CheckOutcome::Pass
  } else {
    CheckOutcome::Fail {
      detail: format!("Cargo {cargo} / flake {flake} differ from HEAD {head}"),
    }
  }
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

fn resolve_pins(dir: &Path) -> Pins {
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
        && !line.contains("LoganBarnett/rust-template")
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
