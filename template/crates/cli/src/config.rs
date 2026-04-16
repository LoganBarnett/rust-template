use clap::Parser;
use rust_template_foundation::config::{
  find_config_file, load_toml, resolve_log_settings, CommonCli,
  CommonConfigFile, ConfigFileError,
};
use rust_template_lib::{LogFormat, LogLevel};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
  #[error("Failed to load configuration file: {0}")]
  File(#[from] ConfigFileError),

  #[error("Configuration validation failed: {0}")]
  Validation(String),
}

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct CliRaw {
  #[command(flatten)]
  pub common: CommonCli,

  /// Example: Name to greet
  #[arg(short, long)]
  pub name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ConfigFileRaw {
  #[serde(flatten)]
  pub common: CommonConfigFile,

  pub name: Option<String>,
}

#[derive(Debug)]
pub struct Config {
  pub log_level: LogLevel,
  pub log_format: LogFormat,
  pub name: String,
}

impl Config {
  pub fn from_cli_and_file(cli: CliRaw) -> Result<Self, ConfigError> {
    let config_file: ConfigFileRaw =
      match find_config_file("rust-template", cli.common.config.as_deref()) {
        Some(path) => load_toml(&path)?,
        None => ConfigFileRaw::default(),
      };

    let (log_level, log_format) = resolve_log_settings(
      cli.common.log_level,
      cli.common.log_format,
      &config_file.common,
    )
    .map_err(ConfigError::Validation)?;

    let name = cli
      .name
      .or(config_file.name)
      .unwrap_or_else(|| "World".to_string());

    Ok(Config {
      log_level,
      log_format,
      name,
    })
  }
}
