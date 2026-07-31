//! Staged configuration for the dependency bumper.
//!
//! Built on the foundation `MergeConfig` convention: each field becomes a
//! `--kebab-case` CLI flag (with an optional config-file and env-var
//! source), and `#[foundation_main]` resolves the layered `Config` before
//! `main` runs.
//!
//! `--report-file` is a `String` (empty means "no report") rather than an
//! `Option<String>` because `MergeConfig` already wraps every field in
//! `Option` for the CLI layer, and a double `Option` parses awkwardly.

use rust_template_dependency_bump_lib::{LogFormat, LogLevel};
use rust_template_foundation::MergeConfig;

#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "dependency-bump")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  /// Changelog file to update; skipped with a notice when absent.
  #[merge_config(default = "\"CHANGELOG.org\".to_string()")]
  pub changelog: String,
  /// TSV report destination (empty = no report file).
  #[merge_config(default = "String::new()")]
  pub report_file: String,
  /// Workspace to bump (default: the current directory).
  #[merge_config(default = "\".\".to_string()")]
  pub workspace_dir: String,
  /// Preview what would move without touching anything.
  #[merge_config(default = "false")]
  pub dry_run: bool,
}
