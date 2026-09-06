//! rust-template-review-cli — review the current change set.
//!
//! Run it when a piece of work is done.  It compares the tree against a base,
//! sends only what it has not already judged to a nested reviewer, and reports
//! what stands.  `--output llm-prompt` renders the same findings as a prompt
//! for a fresh session whose only task is to act on them — an agent that did
//! not write the code, and so has nothing invested in believing it is
//! finished.
//!
//! The exit code follows the convention a linter follows, in either output
//! mode: nothing to report, something to report, or could not look.  A run
//! that found something and a run that broke must not look alike, because a
//! task runner reports both as a failed recipe and the code is all that
//! distinguishes them.

mod config;
mod error;
mod render;

use config::Config;
use error::AppError;
use rust_template_foundation::main as foundation_main;
use std::process::ExitCode;
use tracing::error;

/// The review ran and the changes conform.
const CONFORMS: u8 = 0;
/// The review ran and something stands against the changes.
const FINDINGS: u8 = 1;
/// The review could not be carried out, so the changes were never judged.
const COULD_NOT_RUN: u8 = 2;

#[foundation_main]
pub fn main(config: Config) -> Result<ExitCode, AppError> {
  Ok(reviewed(&config).unwrap_or_else(|failure| {
    error!(%failure, "the review could not run");
    ExitCode::from(COULD_NOT_RUN)
  }))
}

/// The review proper.  Its failure is rendered by the caller rather than
/// returned, since the entry point maps every error to one exit code and this
/// tool needs two.
fn reviewed(config: &Config) -> Result<ExitCode, AppError> {
  // Before the review, not after: a flag the report cannot honour must not
  // cost a reviewer run to discover.
  render::check(config.format, config.column_wrap)?;
  let outcome = rust_template_review_lib::run(&config.request())?;
  render::print(config.format, config.output, config.column_wrap, &outcome)?;
  Ok(ExitCode::from(if render::found_anything(&outcome) {
    FINDINGS
  } else {
    CONFORMS
  }))
}
