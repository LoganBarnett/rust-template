use rust_template_foundation::prelude::{LogFormat, LogLevel};
use rust_template_foundation::MergeConfig;
use std::path::Path;

/// The gate's configuration.  There is nothing to configure in normal use —
/// the hook input arrives on stdin — so beyond the common logging fields the
/// only knobs are the test seams, which let the integration tests drive
/// every branch without a real working tree or a real compile.
///
/// The seams are `String` with an empty default rather than `Option<String>`:
/// `MergeConfig` already wraps every field in `Option` at the CLI layer, and an
/// empty string is the documented "not set" for merged string fields.
#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "review-stop")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  /// Test seam: a file holding a newline-separated working-tree file list,
  /// used in place of the git reads so the gate's behaviour can be exercised
  /// without mutating a real working tree.
  #[merge_config(env, default = "String::new()")]
  pub git_files_file: String,
  /// Test seam: a shell command run in place of the real clippy gate, so the
  /// clippy branch can be driven without a real compile.
  #[merge_config(env, default = "String::new()")]
  pub clippy_cmd: String,
}

impl Config {
  /// The working-tree file-list fixture, when the seam is set.
  pub fn git_files_fixture(&self) -> Option<&Path> {
    Some(self.git_files_file.as_str())
      .filter(|path| !path.is_empty())
      .map(Path::new)
  }

  /// The injected clippy command, when the seam is set.
  pub fn clippy_seam(&self) -> Option<&str> {
    Some(self.clippy_cmd.as_str()).filter(|command| !command.is_empty())
  }
}
