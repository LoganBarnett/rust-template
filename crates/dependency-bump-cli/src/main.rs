//! rust-template-dependency-bump-cli — entry point.
//!
//! `#[foundation_main]` handles CLI parsing, config resolution, and logging
//! init; this file drives the engine and narrates the result.  The process
//! exits non-zero on any engine error, so the scheduled workflow can gate
//! on it.  Run locally, the tool stops at the working tree — review the
//! diff and commit yourself; the scheduled workflow owns branch, PR, and
//! merge.

mod config;

use config::Config;
use rust_template_dependency_bump_lib::{run, DependencyBumpError, RunOptions};
use rust_template_foundation::main as foundation_main;
use std::path::PathBuf;
use std::process::ExitCode;
use thiserror::Error;

#[derive(Debug, Error)]
enum AppError {
  #[error("dependency bump failed: {0}")]
  Bump(#[from] DependencyBumpError),
}

#[foundation_main]
pub fn main(config: Config) -> Result<ExitCode, AppError> {
  let outcome = run(&RunOptions {
    workspace_dir: PathBuf::from(&config.workspace_dir),
    changelog_file: config.changelog.clone(),
    report_file: (!config.report_file.is_empty())
      .then(|| PathBuf::from(&config.report_file)),
    dry_run: config.dry_run,
  })?;

  outcome.held.iter().for_each(|hold| {
    println!("Held (not bumped): {} — {}", hold.package, hold.reason);
  });
  if config.dry_run {
    println!("(--dry-run: nothing touched; cargo's preview is above.)");
  } else if outcome.bumps.is_empty() {
    println!(
      "Nothing moved: every dependency is already as new as its \
       constraint allows."
    );
  } else {
    println!("Landed {} bump(s):", outcome.bumps.len());
    outcome.bumps.iter().for_each(|bump| {
      println!(
        "  {} {} -> {} ({})",
        bump.name, bump.from, bump.to, bump.entry.heading
      );
    });
  }
  Ok(ExitCode::SUCCESS)
}
