use clap::Parser;
use rust_template_foundation::auth::OidcConfig;
use rust_template_foundation::config::{
  find_config_file, load_toml, resolve_log_settings, CommonCli,
  CommonConfigFile, ConfigFileError,
};
use rust_template_lib::{LogFormat, LogLevel};
use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;
use tokio_listener::ListenerAddress;

#[derive(Debug, Error)]
pub enum ConfigError {
  #[error("Failed to load configuration file: {0}")]
  File(#[from] ConfigFileError),

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
  #[command(flatten)]
  pub common: CommonCli,

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
  #[serde(flatten)]
  pub common: CommonConfigFile,

  pub listen: Option<String>,
  pub frontend_path: Option<PathBuf>,
  pub base_url: Option<String>,
  pub oidc_issuer: Option<String>,
  pub oidc_client_id: Option<String>,
  pub oidc_client_secret_file: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Config {
  pub log_level: LogLevel,
  pub log_format: LogFormat,
  pub listen_address: ListenerAddress,
  pub frontend_path: PathBuf,
  pub base_url: String,
  pub oidc: Option<OidcConfig>,
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

    let oidc_issuer = cli.oidc_issuer.or(config_file.oidc_issuer);
    let oidc_client_id = cli.oidc_client_id.or(config_file.oidc_client_id);
    let oidc_secret_file = cli
      .oidc_client_secret_file
      .or(config_file.oidc_client_secret_file);

    let oidc = match (&oidc_issuer, &oidc_client_id) {
      (None, None) if oidc_secret_file.is_none() => None,
      (Some(issuer), Some(client_id)) => {
        let secret_file = oidc_secret_file
          .or_else(credential_secret_path)
          .ok_or_else(|| {
            ConfigError::Validation(
              "oidc_client_secret_file is required when \
                             oidc_issuer and oidc_client_id are set (set it \
                             explicitly or run under systemd with \
                             LoadCredential)"
                .to_string(),
            )
          })?;

        let client_secret = std::fs::read_to_string(&secret_file)
          .map(|s| s.trim().to_string())
          .map_err(|source| ConfigFileError::FileRead {
            path: secret_file,
            source,
          })?;

        Some(OidcConfig {
          issuer: issuer.clone(),
          client_id: client_id.clone(),
          client_secret,
        })
      }
      _ => {
        let mut present = Vec::new();
        let mut missing = Vec::new();
        for (name, val) in [
          ("oidc_issuer", oidc_issuer.is_some()),
          ("oidc_client_id", oidc_client_id.is_some()),
          (
            "oidc_client_secret_file",
            oidc_secret_file.is_some() || credential_secret_path().is_some(),
          ),
        ] {
          if val {
            present.push(name);
          } else {
            missing.push(name);
          }
        }
        return Err(ConfigError::Validation(format!(
          "partial OIDC configuration: set all three fields or \
                     none. present: [{}], missing: [{}]",
          present.join(", "),
          missing.join(", ")
        )));
      }
    };

    Ok(Config {
      log_level,
      log_format,
      listen_address,
      frontend_path,
      base_url,
      oidc,
    })
  }
}

/// Returns the path to the `oidc-client-secret` credential file inside
/// systemd's `CREDENTIALS_DIRECTORY`, if the directory is set and the
/// file exists.
fn credential_secret_path() -> Option<PathBuf> {
  let dir = std::env::var("CREDENTIALS_DIRECTORY").ok()?;
  let path = PathBuf::from(dir).join("oidc-client-secret");
  path.exists().then_some(path)
}
