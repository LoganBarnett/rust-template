use rust_template_foundation::logging::{LogFormat, LogLevel};
use rust_template_foundation::MergeConfig;

#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "example-cli")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  #[merge_config(short, default = "\"World\".to_string()")]
  pub name: String,
}
