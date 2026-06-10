//! Staged configuration for the compliance checker.
//!
//! Built on the foundation `MergeConfig` convention: each field becomes a
//! `--kebab-case` CLI flag (with an optional config-file and env-var source),
//! and `#[foundation_main]` resolves the layered `Config` before `main` runs.
//!
//! `--project` is a `String` (empty means "all spawns") rather than an
//! `Option<String>` because `MergeConfig` already wraps every field in `Option`
//! for the CLI layer, and a double `Option` parses awkwardly.

use rust_template_compliance_lib::{LogFormat, LogLevel};
use rust_template_foundation::MergeConfig;

/// How the report is rendered.
#[derive(
  Debug, Clone, Default, PartialEq, Eq, clap::ValueEnum, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
  /// A colored, human-readable summary.
  #[default]
  Human,
  /// The full report as JSON.
  Json,
}

#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "compliance")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  /// Restrict the run to the spawn of this name (empty = every spawn).
  #[merge_config(default = "String::new()")]
  pub project: String,
  /// Path to the spawn registry (config.json).
  #[merge_config(default = "\"config.json\".to_string()")]
  pub registry: String,
  /// Path to the check manifest (compliance-checks.toml).
  #[merge_config(default = "\"compliance-checks.toml\".to_string()")]
  pub manifest: String,
  /// Template checkout whose HEAD the pins-current check compares against.
  #[merge_config(default = "\".\".to_string()")]
  pub template_dir: String,
  /// Output format: human or json.
  #[merge_config(default = "OutputFormat::Human")]
  pub format: OutputFormat,
}
