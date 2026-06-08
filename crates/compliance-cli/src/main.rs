//! rust-template-compliance-cli — entry point.
//!
//! The `#[foundation_main]` macro handles CLI parsing, config
//! resolution, and logging init.  This file only contains the
//! application-specific logic.

mod config;

use config::Config;
use rust_template_foundation::main as foundation_main;
use std::process::ExitCode;
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
enum AppError {
  #[error("Application execution failed: {0}")]
  Execution(String),
}

#[foundation_main]
pub fn main(config: Config) -> Result<ExitCode, AppError> {
  info!("Hello, {}!", config.name);
  Ok(ExitCode::SUCCESS)
}
