use rust_template_foundation::MergeConfig;
use rust_template_lib::{LogFormat, LogLevel};

#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "rust-template")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  /// Name to greet.
  #[merge_config(short, default = "\"World\".to_string()")]
  pub name: String,
}
