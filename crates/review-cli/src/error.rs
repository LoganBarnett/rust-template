use rust_template_review_lib::ReviewError;
use thiserror::Error;

/// The front-end's failure modes: the review itself, and reporting it.  The
/// engine's errors already name the operation that failed, so this layer adds
/// only which half of the run was in progress.
#[derive(Debug, Error)]
pub enum AppError {
  #[error("the review could not run: {0}")]
  Review(#[from] ReviewError),
  #[error("could not write the report: {0}")]
  ReportWrite(#[from] std::io::Error),
  #[error(
    "--column-wrap {asked} is not available: the reflow is org-fmt's, whose \
     wrap column is fixed at {only}.  Pass --column-wrap {only}, or leave it \
     out for an unwrapped report."
  )]
  WrapWidthUnavailable { asked: u32, only: u32 },
  #[error(
    "--column-wrap needs --format org: org-fmt is what reflows the report and \
     it reads org alone.  Drop --column-wrap, or ask for --format org."
  )]
  WrapNeedsOrg,
}
