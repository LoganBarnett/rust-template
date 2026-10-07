//! rust-template-format-staged-cli: entry point.
//!
//! `#[foundation_main]` handles CLI parsing, config resolution, and logging
//! init.  This file runs the engine and maps its result to an exit code.  A
//! pre-commit hook runs this binary, so a non-zero exit stops the commit.

mod config;

use config::Config;
use rust_template_format_staged_lib::{run, FormatStagedError};
use rust_template_foundation::main as foundation_main;
use std::process::ExitCode;
use thiserror::Error;

#[derive(Debug, Error)]
enum AppError {
  #[error("could not format the staged content: {0}")]
  FormatStaged(#[from] FormatStagedError),
}

#[foundation_main]
pub fn main(_config: Config) -> Result<ExitCode, AppError> {
  run()?;
  Ok(ExitCode::SUCCESS)
}
