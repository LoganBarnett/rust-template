use crate::error::AppError;
use crate::reviewer::Verdict;
use serde::Serialize;
use std::io::{self, Write};

/// What the gate tells Claude Code.  A release prints nothing; a block prints
/// the decision document.  Both exit zero — the hook protocol reads the
/// decision from stdout, not from the exit code.
pub enum Decision {
  Release,
  Block { reason: String },
}

/// The document Claude Code reads back for a block.
#[derive(Serialize)]
struct BlockDocument<'a> {
  decision: &'static str,
  reason: &'a str,
}

impl Decision {
  pub fn emit(&self) -> Result<(), AppError> {
    match self {
      Self::Release => Ok(()),
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
    "clippy reported problems on the Rust changes this turn.  Resolve every \
     warning before ending the turn — this is the same gate CI enforces:\n\n    \
     cargo clippy --workspace --all-targets --all-features -- --deny \
     warnings\n\n{clippy_output}"
  )
}

/// A clean verdict releases; anything else blocks with the findings.
pub fn review_decision(verdict: &Verdict) -> Decision {
  if verdict.passes() {
    Decision::Release
  } else {
    Decision::Block {
      reason: review_reason(verdict),
    }
  }
}

/// Print the review's outcome to stdout for a human — the `--review` path.
/// `None` means nothing changed; an empty verdict is a clean pass; otherwise
/// the findings, in the same rendering a block reason carries.
pub fn print_report(verdict: Option<&Verdict>) -> Result<(), AppError> {
  let mut stdout = io::stdout().lock();
  match verdict {
    None => writeln!(stdout, "No working-tree changes to review."),
    Some(verdict) if verdict.passes() => writeln!(stdout, "No findings."),
    Some(verdict) => write!(stdout, "{}", findings_report(verdict)),
  }
  .map_err(AppError::ReportWrite)
}

/// The findings as both the block reason and the `--review` report print them.
pub fn findings_report(verdict: &Verdict) -> String {
  verdict
    .findings
    .iter()
    .map(|finding| {
      format!(
        "  {}:{}  {} ({})\n      fix: {}\n",
        finding.path,
        finding.line,
        finding.convention,
        finding.document,
        finding.fix
      )
    })
    .collect()
}

fn review_reason(verdict: &Verdict) -> String {
  format!(
    "The review found {} in the working tree:\n\n{}\nAddress every finding.  \
     The gate reviews the tree again when the turn next ends and releases \
     only when the review finds nothing; there is no other way for the turn \
     to end.",
    count(verdict.findings.len()),
    findings_report(verdict)
  )
}

fn count(findings: usize) -> String {
  if findings == 1 {
    "1 finding".to_string()
  } else {
    format!("{findings} findings")
  }
}

/// A gate that cannot run blocks rather than stands aside, and says why.
pub fn failure_reason(error: &AppError) -> String {
  format!(
    "The review gate could not complete: {error}\n\nThe turn cannot end until \
     the gate runs.  Do not work around the gate; if the cause is not yours \
     to fix, report this error to the human verbatim."
  )
}
