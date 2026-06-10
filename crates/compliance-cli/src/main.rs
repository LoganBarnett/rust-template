//! rust-template-compliance-cli — entry point.
//!
//! `#[foundation_main]` handles CLI parsing, config resolution, and logging
//! init; this file drives the engine and renders the result.  The process
//! exits non-zero when any check fails or errors, so the checker can gate CI.

mod config;
mod report;

use config::{Config, OutputFormat};
use rust_template_compliance_lib::{run, ComplianceError, RunOptions};
use rust_template_foundation::main as foundation_main;
use std::path::PathBuf;
use std::process::ExitCode;
use thiserror::Error;

#[derive(Debug, Error)]
enum AppError {
  #[error("compliance run failed: {0}")]
  Run(#[from] ComplianceError),
  #[error("could not render the JSON report: {0}")]
  Json(#[from] serde_json::Error),
}

#[foundation_main]
pub fn main(config: Config) -> Result<ExitCode, AppError> {
  let filter = (!config.project.is_empty()).then(|| config.project.clone());

  let report = run(&RunOptions {
    config_path: PathBuf::from(&config.registry),
    manifest_path: PathBuf::from(&config.manifest),
    template_dir: PathBuf::from(&config.template_dir),
    filter,
  })?;

  match config.format {
    OutputFormat::Human => report::print_human(&report),
    OutputFormat::Json => {
      println!("{}", serde_json::to_string_pretty(&report)?)
    }
  }

  // Phase 2 will add a `--fix` flag here.  The engine intentionally exposes
  // no fix capability yet, so the seam is the absence of one, not a stub.

  Ok(if report.has_failures() {
    ExitCode::FAILURE
  } else {
    ExitCode::SUCCESS
  })
}
