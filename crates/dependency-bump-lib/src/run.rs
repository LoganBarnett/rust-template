//! The engine run: audit, update, re-pin holds, compose, report.
//!
//! Every subprocess the flow needs funnels through here — `cargo update`
//! (and the per-hold re-pin), `cargo audit`, `changelog-roller`, `org-fmt`,
//! and the `git` cleanliness probe — so the pure modules (lockfile, audit,
//! compose, holds) stay unit-testable without a process environment.

use crate::audit::{self, Advisories, AuditProbeError};
use crate::compose::{self, Entry};
use crate::error::DependencyBumpError;
use crate::holds::{self, Hold};
use crate::lockfile::{self, Bump, Snapshot};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a run was asked to do.
#[derive(Debug, Clone)]
pub struct RunOptions {
  /// Workspace to bump (its root holds Cargo.toml and Cargo.lock).
  pub workspace_dir: PathBuf,
  /// Changelog file, relative to the workspace; skipped when absent.
  pub changelog_file: String,
  /// TSV report destination; `None` writes no report.
  pub report_file: Option<PathBuf>,
  /// Preview what would move without touching anything.
  pub dry_run: bool,
}

/// One landed bump with its composed changelog entry.
#[derive(Debug, Clone)]
pub struct AppliedBump {
  pub name: String,
  pub from: String,
  pub to: String,
  pub entry: Entry,
}

/// What a run did.
#[derive(Debug, Clone)]
pub struct BumpOutcome {
  pub bumps: Vec<AppliedBump>,
  pub held: Vec<Hold>,
  pub changelog_updated: bool,
}

/// Runs the bump: classify against the pre-update lockfile, `cargo update`
/// the whole workspace, re-pin the held packages, compose the changelog,
/// and write the report.  The report file is the only machine contract —
/// it is written empty when nothing moved, and not at all on `dry_run`.
pub fn run(options: &RunOptions) -> Result<BumpOutcome, DependencyBumpError> {
  let manifest_path = options.workspace_dir.join("Cargo.toml");
  let lockfile_path = options.workspace_dir.join("Cargo.lock");
  let held = holds::holds(&manifest_path)?;
  held.iter().for_each(|hold| {
    tracing::info!(
      package = %hold.package,
      reason = %hold.reason,
      "hold declared; this package will not advance"
    );
  });
  let advisories = advisories_or_empty(&options.workspace_dir);

  if options.dry_run {
    cargo_update(&options.workspace_dir, true)?;
    return Ok(BumpOutcome {
      bumps: Vec::new(),
      held,
      changelog_updated: false,
    });
  }

  require_clean_lockfile(&options.workspace_dir)?;
  let before = lockfile::snapshot(&lockfile_path)?;
  cargo_update(&options.workspace_dir, false)?;
  repin_held(&options.workspace_dir, &held, &before, &lockfile_path)?;

  let applied =
    lockfile::bumps_between(&before, &lockfile::snapshot(&lockfile_path)?)
      .iter()
      .map(|bump| AppliedBump {
        name: bump.name.clone(),
        from: bump.from.clone(),
        to: bump.to.clone(),
        entry: compose::entry(bump, &advisories),
      })
      .collect::<Vec<_>>();

  let changelog_updated = (!applied.is_empty())
    .then(|| {
      update_changelog(
        &options.workspace_dir,
        &options.changelog_file,
        &applied,
      )
    })
    .transpose()?
    .unwrap_or(false);

  options
    .report_file
    .as_deref()
    .map(|path| write_report(path, &applied))
    .transpose()?;

  Ok(BumpOutcome {
    bumps: applied,
    held,
    changelog_updated,
  })
}

/// The advisory set, or empty when `cargo audit` cannot produce one — an
/// advisory-database outage downgrades classification to Maintenance, it
/// must never block the bump itself.
fn advisories_or_empty(workspace_dir: &Path) -> Advisories {
  audit_probe(workspace_dir).unwrap_or_else(|error| {
    tracing::warn!(
      %error,
      "cargo audit produced no readable report; every bump will be filed \
       under Maintenance"
    );
    Advisories::new()
  })
}

/// Runs `cargo audit --json` and parses its report.  `cargo audit` exits
/// non-zero when it FINDS advisories, so the exit status cannot separate
/// "found some" from "audit broke"; parseable JSON on stdout is the
/// success signal instead, which is why the status goes unexamined here.
fn audit_probe(workspace_dir: &Path) -> Result<Advisories, AuditProbeError> {
  Command::new("cargo")
    .args(["audit", "--json"])
    .current_dir(workspace_dir)
    .stderr(Stdio::null())
    .output()
    .map_err(AuditProbeError::from)
    .and_then(|output| String::from_utf8(output.stdout).map_err(Into::into))
    .and_then(|json| audit::parse_advisories(&json).map_err(Into::into))
}

/// The post-update lockfile diff is the report, so the lockfiles must
/// start clean or the report would claim someone else's changes.
fn require_clean_lockfile(
  workspace_dir: &Path,
) -> Result<(), DependencyBumpError> {
  Command::new("git")
    .args([
      "diff",
      "--quiet",
      "--",
      "Cargo.lock",
      ":(glob)**/Cargo.lock",
    ])
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::LockfileCleanCheckError { source })
    .and_then(|status| match status.code() {
      Some(0) => Ok(()),
      Some(1) => Err(DependencyBumpError::LockfileDirtyError),
      _ => Err(DependencyBumpError::LockfileCleanCheckFailedError { status }),
    })
}

/// Runs `cargo update` across the whole workspace, streaming cargo's own
/// narration through to the user.  With `dry_run`, cargo previews and
/// touches nothing.
fn cargo_update(
  workspace_dir: &Path,
  dry_run: bool,
) -> Result<(), DependencyBumpError> {
  Command::new("cargo")
    .arg("update")
    .args(dry_run.then_some("--dry-run"))
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::CargoUpdateSpawnError { source })
    .and_then(|status| {
      status
        .success()
        .then_some(())
        .ok_or(DependencyBumpError::CargoUpdateFailedError { status })
    })
}

/// Re-pins every held package that `cargo update` advanced back to its
/// pre-update version.  A hold that cargo cannot re-pin fails the run
/// loudly — silently shipping a held bump is the one outcome the hold
/// table exists to prevent.
fn repin_held(
  workspace_dir: &Path,
  held: &[Hold],
  before: &Snapshot,
  lockfile_path: &Path,
) -> Result<(), DependencyBumpError> {
  held.iter().try_for_each(|hold| {
    lockfile::snapshot(lockfile_path).and_then(|current| {
      lockfile::bumps_between(before, &current)
        .into_iter()
        .filter(|bump| bump.name == hold.package)
        .try_for_each(|bump| repin(workspace_dir, &bump, &hold.reason))
    })
  })
}

/// Puts one held package back: `name@new` names the moved instance
/// unambiguously even when several versions coexist in the graph, and
/// `--precise old` restores exactly the pre-update version.
fn repin(
  workspace_dir: &Path,
  bump: &Bump,
  reason: &str,
) -> Result<(), DependencyBumpError> {
  tracing::info!(
    package = %bump.name,
    from = %bump.from,
    to = %bump.to,
    reason,
    "holding package: re-pinning to its pre-update version"
  );
  Command::new("cargo")
    .arg("update")
    .arg("--package")
    .arg(format!("{}@{}", bump.name, bump.to))
    .arg("--precise")
    .arg(&bump.from)
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::HoldRepinSpawnError {
      package: bump.name.clone(),
      source,
    })
    .and_then(|status| {
      status.success().then_some(()).ok_or(
        DependencyBumpError::HoldRepinFailedError {
          package: bump.name.clone(),
          status,
        },
      )
    })
}

/// Inserts one entry per bump and then normalises the file the way a local
/// pre-commit treefmt would (org-fmt wraps long lines), so the composed
/// changelog does not churn a later commit's diff.  A workspace without
/// the changelog file skips composition — the bump commit stands without
/// it.
fn update_changelog(
  workspace_dir: &Path,
  changelog_file: &str,
  bumps: &[AppliedBump],
) -> Result<bool, DependencyBumpError> {
  if !workspace_dir.join(changelog_file).is_file() {
    tracing::warn!(
      changelog = changelog_file,
      "no changelog file here; skipping the changelog entries"
    );
    return Ok(false);
  }
  bumps
    .iter()
    .try_for_each(|bump| insert_entry(workspace_dir, changelog_file, bump))
    .and_then(|()| org_fmt(workspace_dir, changelog_file))
    .map(|()| true)
}

fn insert_entry(
  workspace_dir: &Path,
  changelog_file: &str,
  bump: &AppliedBump,
) -> Result<(), DependencyBumpError> {
  Command::new("changelog-roller")
    .arg("insert-item")
    .arg("--input-file")
    .arg(changelog_file)
    .arg("--heading")
    .arg(bump.entry.heading.to_string())
    .arg("--body")
    .arg(&bump.entry.body)
    .arg("--in-place")
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::ChangelogInsertSpawnError {
      package: bump.name.clone(),
      source,
    })
    .and_then(|status| {
      status.success().then_some(()).ok_or(
        DependencyBumpError::ChangelogInsertFailedError {
          package: bump.name.clone(),
          status,
        },
      )
    })
}

fn org_fmt(
  workspace_dir: &Path,
  changelog_file: &str,
) -> Result<(), DependencyBumpError> {
  let path = workspace_dir.join(changelog_file);
  Command::new("org-fmt")
    .arg("--in-place")
    .arg(changelog_file)
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::OrgFmtSpawnError {
      path: path.clone(),
      source,
    })
    .and_then(|status| {
      status
        .success()
        .then_some(())
        .ok_or(DependencyBumpError::OrgFmtFailedError { path, status })
    })
}

/// Writes the TSV report: `name<TAB>from<TAB>to<TAB>heading` per bump, an
/// empty file when nothing moved.  The report is the scheduled workflow's
/// only contract with this tool; stdout narrates for humans and is never
/// parsed.
fn write_report(
  path: &Path,
  bumps: &[AppliedBump],
) -> Result<(), DependencyBumpError> {
  std::fs::write(
    path,
    bumps
      .iter()
      .map(|bump| {
        format!(
          "{}\t{}\t{}\t{}\n",
          bump.name, bump.from, bump.to, bump.entry.heading
        )
      })
      .collect::<String>(),
  )
  .map_err(|source| DependencyBumpError::ReportWriteError {
    path: path.to_path_buf(),
    source,
  })
}
