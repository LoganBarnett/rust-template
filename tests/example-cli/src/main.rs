use rust_template_foundation::main as foundation_main;
use std::process::ExitCode;
use thiserror::Error;
use tracing::info;

mod config;
use config::Config;

#[derive(Debug, Error)]
enum AppError {
  #[allow(dead_code)]
  #[error("Application execution failed: {0}")]
  Execution(String),
}

#[foundation_main]
pub fn main(config: Config) -> Result<ExitCode, AppError> {
  info!("Hello, {}!", config.name);
  Ok(ExitCode::SUCCESS)
}
