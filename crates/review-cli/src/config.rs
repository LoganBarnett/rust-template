//! The staged configuration.
//!
//! String-typed fields default to empty rather than being `Option<String>`:
//! `MergeConfig` already wraps every field in `Option` at the CLI layer, and an
//! empty string is the documented "not set" for a merged string field.

use crate::error::{AppError, RenameLimitParse};
use rust_template_foundation::MergeConfig;
use rust_template_review_lib::reviewer;
use rust_template_review_lib::{
  DiffMode, LogFormat, LogLevel, Markup, RenameSettings, Request,
};
use std::path::PathBuf;
use std::str::FromStr;

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

/// The rename limit for how many differences are allowed to be considered a
/// rename.  `Inherit` asks for `diff.renameLimit`.  The engine resolves that,
/// so it takes an `Option<usize>` rather than this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "RenameLimitRaw")]
pub enum RenameLimit {
  Inherit,
  Limit(usize),
}

impl FromStr for RenameLimit {
  type Err = RenameLimitParse;

  fn from_str(text: &str) -> Result<Self, Self::Err> {
    if text == "inherit" {
      Ok(Self::Inherit)
    } else {
      text.parse().map(Self::Limit).map_err(RenameLimitParse)
    }
  }
}

/// The raw Rename Limit before validation.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum RenameLimitRaw {
  Count(usize),
  Word(String),
}

impl TryFrom<RenameLimitRaw> for RenameLimit {
  type Error = RenameLimitParse;

  fn try_from(raw: RenameLimitRaw) -> Result<Self, Self::Error> {
    match raw {
      RenameLimitRaw::Count(pairs) => Ok(Self::Limit(pairs)),
      RenameLimitRaw::Word(word) => word.parse(),
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

/// What to do with the findings recorded from earlier passes.
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
pub enum Priors {
  /// Carry each one forward until the file it names changes.
  #[default]
  Keep,
  /// Forget them and judge the files that carried them afresh, so a finding
  /// you disagree with is put to the reviewer again rather than repeated.
  Clear,
}

impl From<Priors> for rust_template_review_lib::Priors {
  fn from(priors: Priors) -> Self {
    match priors {
      Priors::Keep => Self::Keep,
      Priors::Clear => Self::Clear,
    }
  }
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
  /// What to do with the findings recorded from earlier passes: `keep` carries
  /// them forward until the file they name changes; `clear` forgets them and
  /// judges those files afresh.
  #[merge_config(default = "Priors::Keep")]
  pub priors: Priors,
  /// Reflow the report to this many columns; zero, the default, leaves it
  /// unwrapped.  Only `--format org` can be reflowed, and only at 80: the
  /// reflow is org-fmt's, whose wrap column is fixed.
  #[merge_config(default = "0")]
  pub column_wrap: u32,
  /// Model the nested reviewer runs on; empty leaves the CLI's default.
  #[merge_config(default = "String::new()")]
  pub reviewer_model: String,
  /// Agentic turn budget for the nested reviewer, so a review that wanders
  /// cannot run without bound.
  #[merge_config(default = "40")]
  pub reviewer_max_turns: u32,
  /// Wall-clock deadline for the nested reviewer, in seconds.  The review
  /// stops it and fails rather than reporting a tree it never finished
  /// judging.
  #[merge_config(default = "780")]
  pub reviewer_timeout_secs: u64,
  /// Percentage of similarity between files that counts as a rename.  It is
  /// git's `--find-renames`.  100 pairs exact copies only.  `diff.renames =
  /// false` turns pairing off and leaves this idle.
  #[merge_config(default = "50")]
  pub rename_threshold: u8,
  /// Limit of pairs in a rename.  It is git's `-l`.  `inherit` takes
  /// `diff.renameLimit`.  0 passes through unchanged.  Rename detection
  /// follows git and reads it as no limit.  Over the limit, a moved file
  /// ships as a deletion and an addition.
  #[merge_config(default = "RenameLimit::Inherit")]
  pub rename_limit: RenameLimit,
  /// Largest diff or untracked file sent inline, in bytes.  A larger diff goes
  /// by reference for the reviewer to read from the working tree.  The default
  /// is arbitrary.  0 sends everything inline.
  #[merge_config(default = "65536")]
  pub inline_max_bytes: usize,
  /// Where the review record lives; empty means the repository root.
  #[merge_config(default = "String::new()")]
  pub record_file: String,
  /// Test seam: a shell command run in place of the reviewer.  It is fed the
  /// packet on stdin and must print the same JSON envelope `claude --print
  /// --output-format json` does.
  #[merge_config(default = "String::new()")]
  pub reviewer_cmd: String,
  /// Test seam: a file holding a newline-separated change set, used in place
  /// of the git read so the pass can be exercised without a real tree.
  #[merge_config(default = "String::new()")]
  pub git_files_file: String,
}

impl Config {
  /// The engine's view of this configuration, or the argument it cannot honour.
  pub fn request(&self) -> Result<Request, AppError> {
    Ok(Request {
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
      priors: self.priors.into(),
      record_file: set(&self.record_file).map(PathBuf::from),
      git_files_fixture: set(&self.git_files_file).map(PathBuf::from),
      renames: self.renames()?,
      inline_max_bytes: (self.inline_max_bytes != 0)
        .then_some(self.inline_max_bytes),
    })
  }

  /// The rename settings, or the argument the engine cannot take.
  fn renames(&self) -> Result<RenameSettings, AppError> {
    Ok(RenameSettings {
      threshold: (1..=100)
        .contains(&self.rename_threshold)
        .then_some(self.rename_threshold)
        .ok_or(AppError::RenameThresholdOutOfRange {
          asked: self.rename_threshold,
        })?,
      limit: match self.rename_limit {
        RenameLimit::Inherit => None,
        RenameLimit::Limit(pairs) => Some(pairs),
      },
    })
  }
}

/// A merged string field's value, or `None` when it was left unset.
fn set(value: &str) -> Option<String> {
  Some(value.to_string()).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_flag_spells_inherit_or_a_count() {
    assert_eq!("inherit".parse(), Ok(RenameLimit::Inherit));
    assert_eq!("0".parse(), Ok(RenameLimit::Limit(0)));
    assert_eq!("12".parse(), Ok(RenameLimit::Limit(12)));
    assert!("many".parse::<RenameLimit>().is_err());
  }

  #[test]
  fn the_config_file_takes_a_bare_count_or_the_flag_spelling() {
    assert_eq!(
      RenameLimit::try_from(RenameLimitRaw::Count(5)),
      Ok(RenameLimit::Limit(5))
    );
    assert_eq!(
      RenameLimit::try_from(RenameLimitRaw::Word("inherit".to_string())),
      Ok(RenameLimit::Inherit)
    );
  }
}
