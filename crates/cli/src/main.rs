//! rust-template-cli - Command-line application template
//!
//! # LLM Development Guidelines
//! When modifying this code:
//! - Keep configuration logic in config.rs
//! - Keep business logic out of main.rs - use separate modules
//! - Maintain the staged configuration pattern (CliRaw -> ConfigFileRaw -> Config)
//! - Use semantic error types with thiserror - NO anyhow blindly wrapping errors
//! - Add context at each error site explaining WHAT failed and WHY
//! - Keep logging structured and consistent

mod config;

use clap::Parser;
use config::{CliRaw, Config, ConfigError};
use rust_template_lib::init_logging;
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
enum ApplicationError {
  #[error("Failed to load configuration during startup: {0}")]
  ConfigurationLoad(#[from] ConfigError),

  // Example of an execution error - expand with real errors as needed
  #[allow(dead_code)]
  #[error("Application execution failed: {0}")]
  Execution(String),
}

fn main() -> Result<(), ApplicationError> {
  let cli = CliRaw::parse();

  let config = Config::from_cli_and_file(cli).map_err(|e| {
    eprintln!("Configuration error: {}", e);
    ApplicationError::ConfigurationLoad(e)
  })?;

  init_logging(config.log_level, config.log_format);

  info!("Starting rust-template-cli");
  info!("Configuration loaded successfully");

  run(config)?;

  info!("Shutting down rust-template-cli");
  Ok(())
}

fn run(config: Config) -> Result<(), ApplicationError> {
  info!("Hello, {}!", config.name);
  Ok(())
}
