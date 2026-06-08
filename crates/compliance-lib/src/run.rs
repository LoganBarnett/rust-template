//! Orchestration: load inputs, run every check against every spawn in
//! parallel, and collect a [`RunReport`].

use crate::check::{run_check, CheckOutcome, SpawnContext};
use crate::error::ComplianceError;
use crate::manifest::{self, Check};
use crate::provenance;
use crate::registry::{self, Spawn};
use serde::Serialize;
use std::path::PathBuf;

/// Inputs for a compliance run.
#[derive(Debug, Clone)]
pub struct RunOptions {
  /// Path to the spawn registry (`config.json`).
  pub config_path: PathBuf,
  /// Path to the check manifest (`compliance-checks.toml`).
  pub manifest_path: PathBuf,
  /// The template checkout whose `HEAD` the `pins-current` check compares
  /// against.
  pub template_dir: PathBuf,
  /// When set, restrict the run to the single spawn of this name.
  pub filter: Option<String>,
}

/// One check's result within a spawn report.  The outcome is flattened so the
/// JSON shape is `{ id, description, status, detail? | reason? }`.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
  pub id: String,
  pub description: String,
  #[serde(flatten)]
  pub outcome: CheckOutcome,
}

/// Why a spawn was (or was not) checked.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpawnStatus {
  Checked,
  ArchivedSkipped,
  MissingDirSkipped,
}

/// The per-spawn portion of a run.
#[derive(Debug, Clone, Serialize)]
pub struct SpawnReport {
  pub project: String,
  pub dir: String,
  pub status: SpawnStatus,
  pub checks: Vec<CheckResult>,
}

impl SpawnReport {
  fn skipped(project: &str, dir: &str, status: SpawnStatus) -> Self {
    SpawnReport {
      project: project.to_string(),
      dir: dir.to_string(),
      status,
      checks: Vec::new(),
    }
  }

  fn panicked(project: &str) -> Self {
    SpawnReport {
      project: project.to_string(),
      dir: String::new(),
      status: SpawnStatus::Checked,
      checks: vec![CheckResult {
        id: "internal".to_string(),
        description: "check execution".to_string(),
        outcome: CheckOutcome::Error {
          detail: "the check thread for this spawn panicked".to_string(),
        },
      }],
    }
  }
}

/// The full result of a run.
#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
  pub spawns: Vec<SpawnReport>,
}

impl RunReport {
  /// Every check outcome across every checked spawn.
  pub fn outcomes(&self) -> impl Iterator<Item = &CheckOutcome> {
    self
      .spawns
      .iter()
      .flat_map(|spawn| spawn.checks.iter().map(|check| &check.outcome))
  }

  /// Whether any outcome should make the overall run fail.
  pub fn has_failures(&self) -> bool {
    self.outcomes().any(CheckOutcome::is_failure)
  }
}

/// Run every manifest check against every applicable spawn.
pub fn run(opts: &RunOptions) -> Result<RunReport, ComplianceError> {
  let registry = registry::load(&opts.config_path)?;
  let checks = manifest::load(&opts.manifest_path)?;

  // Resolved once and shared across spawns; `pins-current` reports it as an
  // Error outcome if it could not be determined.
  let template_head = crate::pins::template_head(&opts.template_dir);

  let entries: Vec<(&String, &Spawn)> = registry
    .template_spawns
    .iter()
    .filter(|(name, _)| {
      opts
        .filter
        .as_deref()
        .is_none_or(|wanted| wanted == name.as_str())
    })
    .collect();

  tracing::info!(
    spawns = entries.len(),
    checks = checks.len(),
    "running compliance checks"
  );

  // One thread per spawn; spawns are independent and the work is I/O bound.
  let spawns = std::thread::scope(|scope| {
    // The collect is load-bearing, not needless: it spawns every thread
    // before any join, so the spawns actually run concurrently.  Without
    // it, a lazy map would join each handle before spawning the next,
    // serializing the whole run.
    #[allow(clippy::needless_collect)]
    let handles: Vec<(&str, _)> = entries
      .iter()
      .map(|(name, spawn)| {
        let checks = &checks;
        let head = &template_head;
        let name: &str = name;
        let spawn: &Spawn = spawn;
        (name, scope.spawn(move || check_spawn(name, spawn, checks, head)))
      })
      .collect();
    handles
      .into_iter()
      .map(|(name, handle)| {
        handle
          .join()
          .unwrap_or_else(|_| SpawnReport::panicked(name))
      })
      .collect::<Vec<SpawnReport>>()
  });

  Ok(RunReport { spawns })
}

fn check_spawn(
  name: &str,
  spawn: &Spawn,
  checks: &[Check],
  template_head: &Result<String, String>,
) -> SpawnReport {
  if spawn.archived {
    return SpawnReport::skipped(
      name,
      &spawn.dir,
      SpawnStatus::ArchivedSkipped,
    );
  }
  let dir = PathBuf::from(&spawn.dir);
  if !dir.exists() {
    return SpawnReport::skipped(
      name,
      &spawn.dir,
      SpawnStatus::MissingDirSkipped,
    );
  }

  let provenance = provenance::load(&dir);
  let ctx = SpawnContext {
    dir: &dir,
    crates: &spawn.args.crates,
    provenance: &provenance,
    template_head,
  };

  let checks = checks
    .iter()
    .map(|check| CheckResult {
      id: check.id.clone(),
      description: check.description.clone(),
      outcome: run_check(check, &ctx),
    })
    .collect();

  SpawnReport {
    project: name.to_string(),
    dir: spawn.dir.clone(),
    status: SpawnStatus::Checked,
    checks,
  }
}
