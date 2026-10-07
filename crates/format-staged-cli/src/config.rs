//! Staged configuration for format-staged.
//!
//! Built on the foundation `MergeConfig` convention.  The tool takes no
//! options of its own.  A pre-commit hook runs it bare, and what it formats is
//! decided by the index.

use rust_template_format_staged_lib::{LogFormat, LogLevel};
use rust_template_foundation::MergeConfig;

/// Format the staged content of the enclosing repository and re-stage it.  A
/// pre-commit hook runs this.
///
/// Only staged content is formatted and committed.  A hunk left unstaged stays
/// out of the commit.  A file with unstaged changes is not rewritten in the
/// working tree.  The commit then holds the formatted text and the working
/// file keeps the original.  Its unstaged diff reads as both your edit and an
/// undoing of the formatting.  Nothing is lost.  Stage the whole file and
/// commit again to settle it.
#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "format-staged")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
}
