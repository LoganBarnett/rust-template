use crate::error::AppError;
use crate::state::{SessionState, MAX_ROUNDS};
use crate::transcript::{Analysis, Verdict};
use serde::Serialize;
use std::io::{self, Write};
use tracing::warn;

/// The opening phrase of the clippy block reason.  The transcript filters
/// match it verbatim to recognise the block when it comes back as a user turn
/// (see `transcript`).
pub const CLIPPY_BLOCK_MARKER: &str =
  "clippy reported problems on the Rust changes this turn";

/// The sentence both review block reasons close with; matched verbatim by the
/// transcript filters for the same reason as `CLIPPY_BLOCK_MARKER`.
pub const REVIEW_BLOCK_MARKER: &str =
  "This gate releases only when the review reports";

/// The tag wrapping the notification a background subagent posts when it
/// finishes.  It marks a machine-authored user turn in the transcript filters
/// and identifies the notification carriers a verdict may arrive on.
pub const TASK_NOTIFICATION_MARKER: &str = "<task-notification>";

/// What the gate tells Claude Code.  A release prints nothing; a block prints
/// the decision document.  Both exit zero — the hook protocol reads the
/// decision from stdout, not from the exit code.
pub enum Decision {
  /// Allow the stop.  `reviewed` carries the fingerprint to record when the
  /// release follows a verdict (or the give-up); the other release valves
  /// reviewed nothing and record nothing.
  Release {
    reviewed: Option<String>,
  },
  Block {
    reason: String,
  },
}

/// A release that reviewed nothing.
pub const RELEASE: Decision = Decision::Release { reviewed: None };

/// The document Claude Code reads back for a block.
#[derive(Serialize)]
struct BlockDocument<'a> {
  decision: &'static str,
  reason: &'a str,
}

impl Decision {
  pub fn emit(&self) -> Result<(), AppError> {
    match self {
      Self::Release { .. } => Ok(()),
      Self::Block { reason } => serde_json::to_string(&BlockDocument {
        decision: "block",
        reason,
      })
      .map_err(AppError::DecisionSerialize)
      .and_then(|document| {
        writeln!(io::stdout().lock(), "{document}")
          .map_err(AppError::DecisionWrite)
      }),
    }
  }
}

pub fn clippy_reason(clippy_output: &str) -> String {
  format!(
    "{CLIPPY_BLOCK_MARKER}.  Resolve every warning before ending the turn — \
     this is the same gate CI enforces:\n\n    cargo clippy --workspace \
     --all-targets --all-features -- --deny warnings\n\n{clippy_output}"
  )
}

/// The reason for a review block.
pub fn review_reason(verdict: &Verdict) -> String {
  match verdict {
    Verdict::Findings { text } => format!(
      "The template-compliance review reported findings that are not yet \
       resolved:\n\n{text}\n\nAddress every finding, then re-run the \
       template-compliance subagent.  {REVIEW_BLOCK_MARKER} COMPLIANCE: PASS."
    ),
    Verdict::Pass | Verdict::Absent => format!(
      "Files changed this turn, but the template-compliance \
       review has not run.  Invoke the template-compliance subagent (Task \
       tool, subagent_type=\"template-compliance\"), then resolve every \
       finding it reports.  {REVIEW_BLOCK_MARKER} COMPLIANCE: PASS."
    ),
  }
}

/// Bound the consecutive blocks per turn so a finding the assistant cannot
/// resolve does not wedge the session; the count is keyed to the last prompt
/// index, so each turn starts with a fresh budget.
pub fn review_decision(
  session: &SessionState,
  analysis: &Analysis,
  fingerprint: String,
) -> Result<Decision, AppError> {
  let count = session.review_rounds_after(&analysis.turn_key())? + 1;
  Ok(if matches!(analysis.verdict, Verdict::Pass) {
    Decision::Release {
      reviewed: Some(fingerprint),
    }
  } else if count > MAX_ROUNDS {
    // Give up gracefully: release so the turn can end.  The unresolved
    // findings remain visible in the conversation for the human to judge.
    // The fingerprint is recorded here too: the human has now seen those
    // findings, and blocking again next turn on identical content would be
    // nagging rather than gating.  Any new edit re-arms the gate.
    warn!(
      "template-compliance: releasing after {MAX_ROUNDS} unresolved rounds"
    );
    Decision::Release {
      reviewed: Some(fingerprint),
    }
  } else {
    session.write_review_rounds(&analysis.turn_key(), count)?;
    Decision::Block {
      reason: review_reason(&analysis.verdict),
    }
  })
}
