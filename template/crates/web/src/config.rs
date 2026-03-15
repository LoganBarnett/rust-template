use clap::Parser;
use rust_template_lib::{LogFormat, LogLevel};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use thiserror::Error;

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

  #[error("Failed to parse bind address '{address}': {source}")]
  AddressParse {
    address: String,
    #[source]
    source: std::net::AddrParseError,
  },
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

  /// Host/IP address to bind to
  #[arg(long, env = "HOST")]
  pub host: Option<String>,

  /// Port to bind to
  #[arg(long, env = "PORT")]
  pub port: Option<u16>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ConfigFileRaw {
  pub log_level: Option<String>,
  pub log_format: Option<String>,
  pub host: Option<String>,
  pub port: Option<u16>,
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

#[derive(Debug, Clone)]
pub struct Config {
  pub log_level: LogLevel,
  pub log_format: LogFormat,
  pub bind_address: SocketAddr,
}

impl Config {
  pub fn from_cli_and_file(cli: CliRaw) -> Result<Self, ConfigError> {
    let config_file = if let Some(config_path) = &cli.config {
      ConfigFileRaw::from_file(config_path)?
    } else {
      let default_config_path = PathBuf::from("config.toml");
      if default_config_path.exists() {
        ConfigFileRaw::from_file(&default_config_path)?
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

    let host = cli
      .host
      .or(config_file.host)
      .unwrap_or_else(|| "127.0.0.1".to_string());

    let port = cli.port.or(config_file.port).unwrap_or(3000);

    let address_string = format!("{}:{}", host, port);
    let bind_address =
      address_string
        .parse()
        .map_err(|source| ConfigError::AddressParse {
          address: address_string.clone(),
          source,
        })?;

    Ok(Config {
      log_level,
      log_format,
      bind_address,
    })
  }
}
