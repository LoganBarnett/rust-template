use rust_template_foundation::prelude::{LogFormat, LogLevel};
use rust_template_foundation::MergeConfig;
use std::path::Path;

/// The gate's configuration.  The hook input arrives on stdin, so beyond the
/// common logging fields each knob is either a reviewer setting or a test seam,
/// documented on the field.
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
  /// Review the working tree and print the findings instead of acting as the
  /// Stop hook; exits non-zero when there are any.
  #[merge_config(default = "false")]
  pub review: bool,
  /// Model the nested reviewer runs on; empty leaves the CLI's default.
  #[merge_config(env, default = "String::new()")]
  pub reviewer_model: String,
  /// Agentic turn budget for the nested reviewer, so a review that wanders
  /// cannot run past the hook's timeout.
  #[merge_config(env, default = "40")]
  pub reviewer_max_turns: u32,
  /// Wall-clock deadline for the nested reviewer, in seconds.  The gate kills
  /// the reviewer and blocks the turn when it is exceeded, so a slow review
  /// fails closed here rather than being cancelled by Claude Code's hook
  /// timeout — which discards the hook's output and lets the turn end
  /// unreviewed.  Keep it below the Stop hook's `timeout` in settings.json so
  /// this deadline, not the fail-open one, is what fires.
  #[merge_config(env, default = "780")]
  pub reviewer_timeout_secs: u64,
  /// Test seam: a file holding a newline-separated working-tree file list,
  /// used in place of the git reads so the gate's behaviour can be exercised
  /// without mutating a real working tree.
  #[merge_config(env, default = "String::new()")]
  pub git_files_file: String,
  /// Test seam: a shell command run in place of the real clippy gate, so the
  /// clippy branch can be driven without a real compile.
  #[merge_config(env, default = "String::new()")]
  pub clippy_cmd: String,
  /// Test seam: a shell command run in place of the reviewer.  It is fed the
  /// packet on stdin and must print the same JSON envelope `claude --print
  /// --output-format json` does.
  #[merge_config(env, default = "String::new()")]
  pub reviewer_cmd: String,
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

  /// The injected reviewer command, when the seam is set.
  pub fn reviewer_seam(&self) -> Option<&str> {
    Some(self.reviewer_cmd.as_str()).filter(|command| !command.is_empty())
  }

  /// The reviewer's model override, when one is configured.
  pub fn reviewer_model(&self) -> Option<&str> {
    Some(self.reviewer_model.as_str()).filter(|model| !model.is_empty())
  }
}
