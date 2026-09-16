//! Hints for a person at a terminal, the way git points at the command that
//! alters what it just did.  They are logged, not printed: the report is the
//! result, and a hint is commentary on it.  None reaches an automated run,
//! which has no one to act on it and a quieter log for the omission.

use rust_template_review_lib::{Outcome, PriorsApplied};
use std::io::{self, IsTerminal};
use tracing::info;

/// A hint's constant text and the count it reports.
struct Hint {
  message: &'static str,
  findings: usize,
}

/// Say how to get the other treatment of earlier findings, when the choice
/// made a difference this pass.
pub fn priors(outcome: &Outcome) {
  let hint = match outcome {
    Outcome::Reviewed { priors, .. } => priors_hint(*priors),
    Outcome::Unchanged { .. } => None,
  };
  if let Some(shown) = hint.filter(|_| io::stderr().is_terminal()) {
    info!(findings = shown.findings, "{}", shown.message);
  }
}

/// The hint for what a pass did with earlier findings, or `None` when none
/// was in play, so a first pass is not told about a choice that changed
/// nothing.
fn priors_hint(priors: PriorsApplied) -> Option<Hint> {
  match priors {
    PriorsApplied::Kept { recorded } if recorded > 0 => Some(Hint {
      message: "findings from earlier passes were carried forward; pass \
                `--priors clear` (or set `review_priors=clear`) to judge the \
                files that carry them afresh",
      findings: recorded,
    }),
    PriorsApplied::Cleared { forgotten } => Some(Hint {
      message: "findings from earlier passes were forgotten; `--priors keep` \
                (the default) carries them forward instead",
      findings: forgotten,
    }),
    PriorsApplied::Kept { .. } => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_first_pass_is_not_told_about_priors() {
    assert!(priors_hint(PriorsApplied::Kept { recorded: 0 }).is_none());
  }

  #[test]
  fn keeping_points_at_clearing_and_back() {
    let kept =
      priors_hint(PriorsApplied::Kept { recorded: 2 }).expect("a hint");
    assert!(kept.message.contains("--priors clear"), "{}", kept.message);
    assert_eq!(kept.findings, 2);
    let cleared =
      priors_hint(PriorsApplied::Cleared { forgotten: 2 }).expect("a hint");
    assert!(cleared.message.contains("--priors keep"), "{}", cleared.message);
    assert_eq!(cleared.findings, 2);
  }
}
