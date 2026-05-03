use rust_template_foundation::logging::{LogFormat, LogLevel};
use rust_template_foundation::server::runner::{ServerApp, ServerRunConfig};
use rust_template_foundation::{CliApp, MergeConfig};
use tokio_listener::ListenerAddress;

#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "example-server")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  #[merge_config(
    cli_only,
    env = "BASE_URL",
    default = "\"https://example.com\".to_string()"
  )]
  pub base_url: String,
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
