use clap::Parser;
use rust_template_lib::{LogFormat, LogLevel};
use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;
use tokio_listener::ListenerAddress;

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

  #[error("Invalid listen address '{address}': {reason}")]
  InvalidListenAddress {
    address: String,
    reason: &'static str,
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

  /// Address to listen on: host:port for TCP, /path/to.sock for Unix socket,
  /// or sd-listen to inherit a socket from systemd
  #[arg(long, env = "LISTEN")]
  pub listen: Option<String>,

  /// Path to compiled frontend static assets
  #[arg(long, env = "FRONTEND_PATH")]
  pub frontend_path: Option<PathBuf>,

  /// Base URL of the service (e.g. https://example.com), used to construct
  /// the OIDC redirect URI
  #[arg(long, env = "BASE_URL")]
  pub base_url: Option<String>,

  /// OIDC issuer URL (e.g. https://sso.example.com/application/o/myapp)
  #[arg(long, env = "OIDC_ISSUER")]
  pub oidc_issuer: Option<String>,

  /// OIDC client ID
  #[arg(long, env = "OIDC_CLIENT_ID")]
  pub oidc_client_id: Option<String>,

  /// Path to a file containing the OIDC client secret
  #[arg(long, env = "OIDC_CLIENT_SECRET_FILE")]
  pub oidc_client_secret_file: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ConfigFileRaw {
  pub log_level: Option<String>,
  pub log_format: Option<String>,
  pub listen: Option<String>,
  pub frontend_path: Option<PathBuf>,
  pub base_url: Option<String>,
  pub oidc_issuer: Option<String>,
  pub oidc_client_id: Option<String>,
  pub oidc_client_secret_file: Option<PathBuf>,
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
  pub listen_address: ListenerAddress,
  pub frontend_path: PathBuf,
  pub base_url: String,
  pub oidc_issuer: String,
  pub oidc_client_id: String,
  pub oidc_client_secret: String,
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

    let listen_str = cli
      .listen
      .or(config_file.listen)
      .unwrap_or_else(|| "127.0.0.1:3000".to_string());

    let listen_address =
      listen_str.parse::<ListenerAddress>().map_err(|reason| {
        ConfigError::InvalidListenAddress {
          address: listen_str.clone(),
          reason,
        }
      })?;

    let frontend_path = cli
      .frontend_path
      .or(config_file.frontend_path)
      .unwrap_or_else(|| PathBuf::from("frontend/public"));

    let base_url = cli.base_url.or(config_file.base_url).ok_or_else(|| {
      ConfigError::Validation("base_url is required".to_string())
    })?;

    let oidc_issuer =
      cli.oidc_issuer.or(config_file.oidc_issuer).ok_or_else(|| {
        ConfigError::Validation("oidc_issuer is required".to_string())
      })?;

    let oidc_client_id = cli
      .oidc_client_id
      .or(config_file.oidc_client_id)
      .ok_or_else(|| {
        ConfigError::Validation("oidc_client_id is required".to_string())
      })?;

    let secret_file = cli
      .oidc_client_secret_file
      .or(config_file.oidc_client_secret_file)
      .ok_or_else(|| {
        ConfigError::Validation(
          "oidc_client_secret_file is required".to_string(),
        )
      })?;

    let oidc_client_secret = std::fs::read_to_string(&secret_file)
      .map(|s| s.trim().to_string())
      .map_err(|source| ConfigError::FileRead {
        path: secret_file,
        source,
      })?;

    Ok(Config {
      log_level,
      log_format,
      listen_address,
      frontend_path,
      base_url,
      oidc_issuer,
      oidc_client_id,
      oidc_client_secret,
    })
  }
}
