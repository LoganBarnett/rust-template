//! rust-template-review-stop — the code-review Stop hook's gate.
//!
//! Claude Code runs this binary (through `.claude/hooks/review-stop.sh`) each
//! time a turn is about to end.  This is not a "please remember to review"
//! nudge — it is deterministic and not skippable.  Whenever the working tree
//! holds un-reviewed changes, the gate blocks the turn from ending until the
//! template-compliance subagent has run *and reported* COMPLIANCE: PASS.  When
//! any of those changes is a Rust source file, it first blocks until the
//! workspace passes the same clippy gate CI enforces, so a lint failure never
//! reaches the reviewer.
//!
//! The review itself is the native subagent (a Task the assistant invokes);
//! the gate otherwise inspects the git working tree and the transcript.  The
//! one subprocess it runs is clippy — objective and deterministic, so no model
//! discretion — and never a nested `claude`.
//!
//! Convergence: each time the assistant addresses findings and re-runs the
//! reviewer, the next Stop re-reads the verdict and releases once it is PASS.
//! A bounded round cap keeps a finding the assistant genuinely cannot resolve
//! from wedging the session — after the cap the gate releases and the
//! unresolved findings stand in the conversation for the human.
//!
//! The decision travels on stdout; diagnostics go to stderr so stdout stays
//! protocol-clean.

mod clippy;
mod config;
mod decision;
mod error;
mod fingerprint;
mod hook_input;
mod state;
mod transcript;
mod worktree;

use clippy::ClippyOutcome;
use config::Config;
use decision::{Decision, RELEASE};
use error::AppError;
use hook_input::HookInput;
use rust_template_foundation::main as foundation_main;
use state::SessionState;
use std::path::Path;
use std::process::ExitCode;
use tracing::debug;
use transcript::Analysis;

#[foundation_main]
pub fn main(config: Config) -> Result<ExitCode, AppError> {
  let input = HookInput::from_stdin()?;
  let session = SessionState::for_session(input.session_id());
  let decision = gate(&config, &input, &session)?;
  session.settle(&decision)?;
  decision.emit()?;
  Ok(ExitCode::SUCCESS)
}

/// The release valves come first, cheapest to dearest, so a turn with nothing
/// to review never pays for a compile.
fn gate(
  config: &Config,
  input: &HookInput,
  session: &SessionState,
) -> Result<Decision, AppError> {
  if !worktree::inside_git_work_tree() {
    Ok(released("not inside a git work tree"))
  } else if input.plan_mode() {
    Ok(released("plan mode is read-only"))
  } else {
    input.existing_transcript_path().map_or_else(
      || Ok(released("no transcript to read")),
      |transcript_path| worktree_decision(config, session, transcript_path),
    )
  }
}

/// The decision for the working tree's changes: nothing changed, or nothing
/// new since the last released review, releases; otherwise clippy and the
/// review decide.
fn worktree_decision(
  config: &Config,
  session: &SessionState,
  transcript_path: &Path,
) -> Result<Decision, AppError> {
  // Reading git (rather than reconstructing edits from the transcript) catches
  // edits made through the shell, not just the editing tools.
  let qualifying =
    worktree::qualifying(worktree::changed_files(config.git_files_fixture())?);
  if qualifying.files.is_empty() {
    Ok(released("no working-tree changes"))
  } else {
    let fingerprint = fingerprint::of(&qualifying.files)?;
    if session.reviewed_fingerprint()?.as_deref() == Some(fingerprint.as_str())
    {
      Ok(released("working tree unchanged since the last released review"))
    } else {
      let (clippy_outcome, analysis) =
        concurrently(config, session, qualifying.has_rust, transcript_path)?;
      match clippy_outcome {
        Some(ClippyOutcome::Block(reason)) => Ok(Decision::Block { reason }),
        _ => decision::review_decision(session, &analysis, fingerprint),
      }
    }
  }
}

/// Log why the gate stands aside, then release.
fn released(reason: &str) -> Decision {
  debug!("{reason}; releasing");
  RELEASE
}

/// The clippy gate and the transcript analysis touch disjoint inputs — the
/// compiler and the transcript file — and clippy dominates the wall clock, so
/// running them concurrently makes the analysis effectively free.
fn concurrently(
  config: &Config,
  session: &SessionState,
  has_rust: bool,
  transcript_path: &Path,
) -> Result<(Option<ClippyOutcome>, Analysis), AppError> {
  std::thread::scope(|scope| {
    let clippy_handle = scope
      .spawn(|| has_rust.then(|| clippy::gate(config, session)).transpose());
    let analysis = transcript::analyze(transcript_path);
    Ok((
      clippy_handle.join().map_err(|payload| {
        AppError::GateThreadPanicked {
          task: "clippy",
          detail: error::panic_detail(payload.as_ref()),
        }
      })??,
      analysis?,
    ))
  })
}
