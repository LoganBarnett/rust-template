//! rust-template-review-stop — the code-review Stop hook's gate.
//!
//! Claude Code runs this binary (through `.claude/hooks/review-stop.sh`) each
//! time a turn is about to end.  Whenever the working tree holds un-reviewed
//! changes, the gate blocks the turn from ending until a review it runs itself
//! finds nothing to report.  When any of those changes is a Rust source file,
//! it first blocks until the workspace passes the same clippy gate CI enforces,
//! so a lint failure never reaches the reviewer.
//!
//! The gate owns the review end to end: it assembles the change set together
//! with the conventions as committed at `HEAD`, hands them to a nested `claude`
//! run whose prompt is built into this binary, and reads back a structured
//! verdict.  Nothing the agent under review writes reaches the reviewer as
//! instruction — its diff is there as content to judge — so nothing it writes
//! can narrow the scope.  Verdicts are cached against the exact content they
//! judged.
//!
//! There is no release valve.  Findings block until the tree passes, and a gate
//! that cannot run blocks with the error; the human's interrupt is the one way
//! out the agent cannot take.
//!
//! The decision travels on stdout; diagnostics go to stderr so stdout stays
//! protocol-clean.

mod cache;
mod clippy;
mod config;
mod decision;
mod error;
mod fingerprint;
mod hook_input;
mod path_lookup;
mod prompt;
mod reviewer;
mod worktree;

use cache::Entry;
use config::Config;
use decision::Decision;
use error::AppError;
use hook_input::HookInput;
use reviewer::Verdict;
use rust_template_foundation::main as foundation_main;
use std::process::ExitCode;
use tracing::{debug, error};
use worktree::Qualifying;

#[foundation_main]
pub fn main(config: Config) -> Result<ExitCode, AppError> {
  if config.review {
    report(&config)
  } else {
    hook(&config)
  }
}

/// The hook protocol: a release prints nothing, a block prints the decision
/// document, and both exit zero.  A gate failure is a block too — standing
/// aside on failure would be a release the agent could reach by breaking the
/// gate.
fn hook(config: &Config) -> Result<ExitCode, AppError> {
  HookInput::from_stdin()
    .and_then(|input| gate(config, &input))
    .unwrap_or_else(|failure| {
      error!(%failure, "the gate could not complete; blocking the turn");
      Decision::Block {
        reason: decision::failure_reason(&failure),
      }
    })
    .emit()?;
  Ok(ExitCode::SUCCESS)
}

/// The release valves that review nothing come first, cheapest to dearest.
fn gate(config: &Config, input: &HookInput) -> Result<Decision, AppError> {
  if std::env::var_os(reviewer::NESTED_ENV).is_some() {
    Ok(released("this is the nested reviewer's own run"))
  } else if !worktree::inside_git_work_tree() {
    Ok(released("not inside a git work tree"))
  } else if input.plan_mode() {
    Ok(released("plan mode is read-only"))
  } else {
    tree_decision(config)
  }
}

/// Nothing changed releases; a cached verdict decides without a review;
/// otherwise clippy and then a fresh review decide.
fn tree_decision(config: &Config) -> Result<Decision, AppError> {
  let qualifying = changes(config)?;
  if qualifying.files.is_empty() {
    Ok(released("no working-tree changes"))
  } else {
    let entry = Entry::current(&qualifying.files)?;
    entry.verdict()?.map_or_else(
      || fresh_decision(config, &qualifying, &entry),
      |verdict| {
        debug!("deciding on the cached verdict for this tree");
        Ok(decision::review_decision(&verdict))
      },
    )
  }
}

fn fresh_decision(
  config: &Config,
  qualifying: &Qualifying,
  entry: &Entry,
) -> Result<Decision, AppError> {
  qualifying
    .has_rust
    .then(|| clippy::block_reason(config))
    .transpose()?
    .flatten()
    .map_or_else(
      || {
        fresh_verdict(config, entry)
          .map(|verdict| decision::review_decision(&verdict))
      },
      |reason| Ok(Decision::Block { reason }),
    )
}

fn fresh_verdict(config: &Config, entry: &Entry) -> Result<Verdict, AppError> {
  let verdict = reviewer::review(config, &reviewer::packet(entry.head())?)?;
  entry.store(&verdict)?;
  Ok(verdict)
}

/// The working tree's changes come from git, so an edit made through the shell
/// counts the same as one made through the editing tools.
fn changes(config: &Config) -> Result<Qualifying, AppError> {
  worktree::changed_files(config.git_files_fixture()).map(worktree::qualifying)
}

/// Log why the gate stands aside, then release.
fn released(reason: &str) -> Decision {
  debug!("{reason}; releasing");
  Decision::Release
}

/// `--review`: the same review the hook runs, reported on stdout for a human,
/// with the exit code carrying the verdict.
fn report(config: &Config) -> Result<ExitCode, AppError> {
  let verdict = review_verdict(config)?;
  decision::print_report(verdict.as_ref())?;
  Ok(if verdict.is_some_and(|found| !found.passes()) {
    ExitCode::FAILURE
  } else {
    ExitCode::SUCCESS
  })
}

/// The verdict for the working tree, or `None` when nothing changed.  A cached
/// verdict is reused; otherwise the review runs.
fn review_verdict(config: &Config) -> Result<Option<Verdict>, AppError> {
  let qualifying = changes(config)?;
  if qualifying.files.is_empty() {
    Ok(None)
  } else {
    let entry = Entry::current(&qualifying.files)?;
    entry
      .verdict()?
      .map_or_else(|| fresh_verdict(config, &entry), Ok)
      .map(Some)
  }
}
