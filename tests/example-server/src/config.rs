use clap::Parser;
use rust_template_foundation::config::{
  find_config_file, load_toml, resolve_log_settings, CommonCli,
  CommonConfigFile,
};
use rust_template_foundation::logging::{LogFormat, LogLevel};
use rust_template_foundation::server::runner::{
  CliApp, ServerApp, ServerRunConfig,
};
use serde::Deserialize;
use thiserror::Error;
use tokio_listener::ListenerAddress;

#[derive(Debug, Error)]
pub enum ConfigError {
  #[error("Failed to load configuration file: {0}")]
  File(#[from] rust_template_foundation::config::ConfigFileError),

  #[error("Configuration validation failed: {0}")]
  Validation(String),
}

#[derive(Debug, Parser)]
#[command(name = "example-server", version, about)]
pub struct CliRaw {
  #[command(flatten)]
  pub common: CommonCli,

  #[arg(long, env = "BASE_URL", default_value = "https://example.com")]
  pub base_url: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct ConfigFileRaw {
  #[serde(flatten)]
  pub common: CommonConfigFile,
}

#[derive(Debug, Clone)]
pub struct Config {
  pub log_level: LogLevel,
  pub log_format: LogFormat,
  pub base_url: String,
}

impl CliApp for Config {
  type CliArgs = CliRaw;
  type Error = ConfigError;

  fn app_name() -> &'static str {
    "example-server"
  }

  fn from_cli(cli: CliRaw) -> Result<Self, ConfigError> {
    let config_file: ConfigFileRaw =
      match find_config_file("example-server", cli.common.config.as_deref()) {
        Some(path) => load_toml(&path)?,
        None => ConfigFileRaw::default(),
      };

    let (log_level, log_format) = resolve_log_settings(
      cli.common.log_level,
      cli.common.log_format,
      &config_file.common,
    )
    .map_err(ConfigError::Validation)?;

    Ok(Config {
      log_level,
      log_format,
      base_url: cli.base_url,
    })
  }

  fn log_level(&self) -> LogLevel {
    self.log_level
  }

  fn log_format(&self) -> LogFormat {
    self.log_format
  }
}

impl ServerApp for Config {
  fn server_run_configs(&self) -> Vec<ServerRunConfig> {
    vec![ServerRunConfig {
      app_name: Self::app_name().to_string(),
      listen_address: "127.0.0.1:3000".parse::<ListenerAddress>().unwrap(),
      frontend_path: None,
      base_url: self.base_url.clone(),
      oidc: None,
    }]
  }
}
