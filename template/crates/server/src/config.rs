use rust_template_foundation::auth::OidcConfig;
use rust_template_foundation::config::{
  credential_secret_path, ConfigFileError,
};
use rust_template_foundation::server::runner::{ServerApp, ServerRunConfig};
use rust_template_foundation::MergeConfig;
use rust_template_lib::{LogFormat, LogLevel};
use serde::Deserialize;
use std::path::PathBuf;
use tokio_listener::ListenerAddress;

/// OIDC CLI arguments, flattened into the generated `CliRaw`.
#[derive(Debug, clap::Args)]
pub struct OidcCliFields {
  /// OIDC issuer URL
  /// (e.g. https://sso.example.com/application/o/myapp).
  #[arg(long, env = "OIDC_ISSUER")]
  pub oidc_issuer: Option<String>,

  /// OIDC client ID.
  #[arg(long, env = "OIDC_CLIENT_ID")]
  pub oidc_client_id: Option<String>,

  /// Path to a file containing the OIDC client secret.
  #[arg(long, env = "OIDC_CLIENT_SECRET_FILE")]
  pub oidc_client_secret_file: Option<PathBuf>,
}

/// OIDC config file fields, flattened into the generated
/// `ConfigFileRaw`.
#[derive(Debug, Deserialize, Default)]
pub struct OidcFileFields {
  pub oidc_issuer: Option<String>,
  pub oidc_client_id: Option<String>,
  pub oidc_client_secret_file: Option<PathBuf>,
}

#[derive(Debug, Clone, MergeConfig)]
#[merge_config(
  app_name = "rust-template",
  extra_cli = "OidcCliFields",
  extra_file = "OidcFileFields"
)]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  /// Address to listen on: host:port for TCP, /path/to.sock for
  /// Unix socket, or sd-listen to inherit from systemd.
  #[merge_config(
    name = "listen",
    env = "LISTEN",
    default = "\"127.0.0.1:3000\".to_string()",
    parse
  )]
  pub listen_address: ListenerAddress,
  /// Path to compiled frontend static assets.
  #[merge_config(
    env = "FRONTEND_PATH",
    default = "std::path::PathBuf::from(\"frontend/public\")"
  )]
  pub frontend_path: PathBuf,
  /// Base URL of the service (e.g. https://example.com), used to
  /// construct the OIDC redirect URI.
  #[merge_config(env = "BASE_URL", required)]
  pub base_url: String,
  #[merge_config(skip)]
  pub oidc: Option<OidcConfig>,
}

impl std::fmt::Display for Config {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "Config(listen={})", self.listen_address)
  }
}

impl ServerApp for Config {
  fn server_run_configs(&self) -> Vec<ServerRunConfig> {
    vec![ServerRunConfig {
      app_name: Self::app_name().to_string(),
      listen_address: self.listen_address.clone(),
      frontend_path: Some(self.frontend_path.clone()),
      base_url: self.base_url.clone(),
      oidc: self.oidc.clone(),
    }]
  }
}

impl Config {
  fn resolve_oidc(
    cli: &CliRaw,
    file: &ConfigFileRaw,
  ) -> Result<Option<OidcConfig>, ConfigError> {
    let oidc_issuer = cli
      .extra
      .oidc_issuer
      .clone()
      .or_else(|| file.extra.oidc_issuer.clone());
    let oidc_client_id = cli
      .extra
      .oidc_client_id
      .clone()
      .or_else(|| file.extra.oidc_client_id.clone());
    let oidc_secret_file = cli
      .extra
      .oidc_client_secret_file
      .clone()
      .or_else(|| file.extra.oidc_client_secret_file.clone());

    match (&oidc_issuer, &oidc_client_id) {
      (None, None) if oidc_secret_file.is_none() => Ok(None),
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

        Ok(Some(OidcConfig {
          issuer: issuer.clone(),
          client_id: client_id.clone(),
          client_secret,
        }))
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
        Err(ConfigError::Validation(format!(
          "partial OIDC configuration: set all three fields \
           or none. present: [{}], missing: [{}]",
          present.join(", "),
          missing.join(", ")
        )))
      }
    }
  }
}
