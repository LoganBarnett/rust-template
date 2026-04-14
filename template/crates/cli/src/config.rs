use clap::Parser;
use rust_template_lib::{LogFormat, LogLevel};
use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;

/// Resolve `$XDG_CONFIG_HOME/rust-template`, falling back to
/// `$HOME/.config/rust-template` when the variable is unset.
fn xdg_config_dir() -> Option<PathBuf> {
  std::env::var_os("XDG_CONFIG_HOME")
    .map(PathBuf::from)
    .or_else(|| home::home_dir().map(|h| h.join(".config")))
    .map(|d| d.join("rust-template"))
}

#[derive(Debug, Error)]
pub enum ConfigError {
  #[error(
    "Failed to read configuration file at {path:?} during startup: {source}"
  )]
  FileRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },

  #[error("Failed to parse configuration file at {path:?}: {source}")]
  Parse {
    path: PathBuf,
    #[source]
    source: toml::de::Error,
  },

  #[error("Configuration validation failed: {0}")]
  Validation(String),
}

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct CliRaw {
  /// Log level (trace, debug, info, warn, error)
  #[arg(long, env = "LOG_LEVEL")]
  pub log_level: Option<String>,

  /// Log format (text, json)
  #[arg(long, env = "LOG_FORMAT")]
  pub log_format: Option<String>,

  /// Path to configuration file
  #[arg(short, long, env = "CONFIG_FILE")]
  pub config: Option<PathBuf>,

  /// Example: Name to greet
  #[arg(short, long)]
  pub name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ConfigFileRaw {
  pub log_level: Option<String>,
  pub log_format: Option<String>,
  pub name: Option<String>,
}

impl ConfigFileRaw {
  pub fn from_file(path: &PathBuf) -> Result<Self, ConfigError> {
    let contents = std::fs::read_to_string(path).map_err(|source| {
      ConfigError::FileRead {
        path: path.clone(),
        source,
      }
    })?;

    let config: ConfigFileRaw =
      toml::from_str(&contents).map_err(|source| ConfigError::Parse {
        path: path.clone(),
        source,
      })?;

    Ok(config)
  }
}

#[derive(Debug)]
pub struct Config {
  pub log_level: LogLevel,
  pub log_format: LogFormat,
  pub name: String,
}

impl Config {
  pub fn from_cli_and_file(cli: CliRaw) -> Result<Self, ConfigError> {
    let config_file = if let Some(config_path) = &cli.config {
      ConfigFileRaw::from_file(config_path)?
    } else {
      let cwd = PathBuf::from("config.toml");
      let xdg = xdg_config_dir().map(|d| d.join("config.toml"));

      if cwd.exists() {
        ConfigFileRaw::from_file(&cwd)?
      } else if let Some(ref xdg_path) = xdg.filter(|p| p.exists()) {
        ConfigFileRaw::from_file(xdg_path)?
      } else {
        ConfigFileRaw::default()
      }
    };

    let log_level_str = cli
      .log_level
      .or(config_file.log_level)
      .unwrap_or_else(|| "info".to_string());

    let log_level = log_level_str
      .parse::<LogLevel>()
      .map_err(|e| ConfigError::Validation(e.to_string()))?;

    let log_format_str = cli
      .log_format
      .or(config_file.log_format)
      .unwrap_or_else(|| "text".to_string());

    let log_format = log_format_str
      .parse::<LogFormat>()
      .map_err(|e| ConfigError::Validation(e.to_string()))?;

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
