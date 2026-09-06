//! The staged configuration.
//!
//! String-typed fields default to empty rather than being `Option<String>`:
//! `MergeConfig` already wraps every field in `Option` at the CLI layer, and an
//! empty string is the documented "not set" for a merged string field.

use rust_template_foundation::MergeConfig;
use rust_template_review_lib::reviewer;
use rust_template_review_lib::{
  DiffMode, LogFormat, LogLevel, Markup, Request,
};
use std::path::PathBuf;

/// What the review compares against.  `Auto` is a request, not a comparison:
/// resolving it is what turns it into one, which is why the engine takes an
/// `Option<DiffMode>` rather than this type.
#[derive(
  Debug,
  Clone,
  Copy,
  Default,
  PartialEq,
  Eq,
  clap::ValueEnum,
  serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum DiffRequest {
  /// Decide from the tree: uncommitted work if there is any, else the branch.
  #[default]
  Auto,
  /// The working tree against `HEAD`.
  Head,
  /// The branch against where it left the default branch.
  Branch,
}

/// The markup the report is written in.  It reaches the reviewer as well as
/// the renderer: the reviewer is told to write its prose in this markup, so a
/// finding arrives already marked up rather than being converted afterwards.
#[derive(
  Debug,
  Clone,
  Copy,
  Default,
  PartialEq,
  Eq,
  clap::ValueEnum,
  serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
  /// Prose with no markup, for a terminal or a grep.
  Plain,
  /// Light Markdown, with findings as a task list.
  #[default]
  Md,
  /// Light Org, with findings as checkbox items.
  Org,
}

impl From<Format> for Markup {
  fn from(format: Format) -> Self {
    match format {
      Format::Plain => Self::Plain,
      Format::Md => Self::Markdown,
      Format::Org => Self::Org,
    }
  }
}

/// How the findings are rendered.
#[derive(
  Debug,
  Clone,
  Copy,
  Default,
  PartialEq,
  Eq,
  clap::ValueEnum,
  serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Output {
  /// A report for the person who ran the review.
  #[default]
  Human,
  /// A prompt for a fresh session whose task is to act on the findings.
  LlmPrompt,
}

#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "review")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  /// What the review compares against.
  #[merge_config(default = "DiffRequest::Auto")]
  pub diff: DiffRequest,
  /// How the findings are rendered.
  #[merge_config(default = "Output::Human")]
  pub output: Output,
  /// The markup the report and the reviewer's prose are written in.
  #[merge_config(default = "Format::Md")]
  pub format: Format,
  /// Reflow the report to this many columns; zero, the default, leaves it
  /// unwrapped.  Only `--format org` can be reflowed, and only at 80: the
  /// reflow is org-fmt's, whose wrap column is fixed.
  #[merge_config(env, default = "0")]
  pub column_wrap: u32,
  /// Model the nested reviewer runs on; empty leaves the CLI's default.
  #[merge_config(env, default = "String::new()")]
  pub reviewer_model: String,
  /// Agentic turn budget for the nested reviewer, so a review that wanders
  /// cannot run without bound.
  #[merge_config(env, default = "40")]
  pub reviewer_max_turns: u32,
  /// Wall-clock deadline for the nested reviewer, in seconds.  The review
  /// stops it and fails rather than reporting a tree it never finished
  /// judging.
  #[merge_config(env, default = "780")]
  pub reviewer_timeout_secs: u64,
  /// Where the review record lives; empty means the repository root.
  #[merge_config(env, default = "String::new()")]
  pub record_file: String,
  /// Test seam: a shell command run in place of the reviewer.  It is fed the
  /// packet on stdin and must print the same JSON envelope `claude --print
  /// --output-format json` does.
  #[merge_config(env, default = "String::new()")]
  pub reviewer_cmd: String,
  /// Test seam: a file holding a newline-separated change set, used in place
  /// of the git read so the pass can be exercised without a real tree.
  #[merge_config(env, default = "String::new()")]
  pub git_files_file: String,
}

impl Config {
  /// The engine's view of this configuration.
  pub fn request(&self) -> Request {
    Request {
      diff: match self.diff {
        DiffRequest::Auto => None,
        DiffRequest::Head => Some(DiffMode::Head),
        DiffRequest::Branch => Some(DiffMode::Branch),
      },
      reviewer: reviewer::Options {
        model: set(&self.reviewer_model),
        max_turns: self.reviewer_max_turns,
        timeout_secs: self.reviewer_timeout_secs,
        markup: self.format.into(),
        command: set(&self.reviewer_cmd),
      },
      record_file: set(&self.record_file).map(PathBuf::from),
      git_files_fixture: set(&self.git_files_file).map(PathBuf::from),
    }
  }
}

/// A merged string field's value, or `None` when it was left unset.
fn set(value: &str) -> Option<String> {
  Some(value.to_string()).filter(|text| !text.is_empty())
}
