//! One pass of the review: work out what to compare, what still needs
//! looking at, and what stands once the reviewer has spoken.

use crate::base::{self, Base, DiffMode};
use crate::error::ReviewError;
use crate::fingerprint;
use crate::markup::Markup;
use crate::record::{self, Record, Standing};
use crate::reviewer::{self, Scope};
use crate::worktree::{ChangeSet, Worktree};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// What to review and how.
#[derive(Debug, Clone)]
pub struct Request {
  /// The comparison to run, or `None` to decide from the tree.
  pub diff: Option<DiffMode>,
  pub reviewer: reviewer::Options,
  /// Where the record lives, or `None` for the repository root.
  pub record_file: Option<PathBuf>,
  /// A file holding a newline-separated change set, used in place of the git
  /// read so tests can drive the pass without a real tree.
  pub git_files_fixture: Option<PathBuf>,
}

/// What a pass concluded.
#[derive(Debug)]
pub enum Outcome {
  /// The comparison covers nothing.
  Unchanged { base: Base },
  Reviewed {
    base: Base,
    standing: Standing,
    /// The paths this pass sent to the reviewer; empty when every one of them
    /// was already judged at its current content.
    reviewed: Vec<String>,
  },
}

pub fn run(request: &Request) -> Result<Outcome, ReviewError> {
  let worktree = Worktree::open()?;
  let base = base::resolve(&worktree, request.diff)?;
  let path = request
    .record_file
    .clone()
    .unwrap_or_else(|| record::path_for(worktree.root()));
  let changed =
    base.changed(&worktree, request.git_files_fixture.as_deref())?;
  if changed.paths.is_empty() {
    Record::remove(&path).map(|()| Outcome::Unchanged { base })
  } else {
    judge(request, &worktree, &path, base, &changed)
  }
}

/// Which bucket of the record this pass reads and writes.  The markup is part
/// of it because the reviewer writes its prose in that markup, so a finding
/// cached under one cannot be rendered as another.
fn record_key(base: &Base, markup: Markup) -> String {
  format!("{}:{}", base.key(), markup.label())
}

/// One pass over a change set that has something in it: reuse whatever the
/// record already covers at its current content, and review the rest.
fn judge(
  request: &Request,
  worktree: &Worktree,
  path: &Path,
  base: Base,
  changed: &ChangeSet,
) -> Result<Outcome, ReviewError> {
  info!(
    scope = %base.describe(),
    files = changed.paths.len(),
    "reviewing the change set"
  );
  let blobs = fingerprint::blobs(worktree, &changed.paths)?;
  let mut recorded = Record::load(path)?;
  let key = record_key(&base, request.reviewer.markup);
  let partition = recorded.partition(&key, &blobs);
  if partition.stale.is_empty() {
    info!(
      reused = partition.reused.len(),
      "every changed path was already judged at its current content; \
       no reviewer needed"
    );
    Ok(Outcome::Reviewed {
      standing: recorded.standing(&key),
      reviewed: Vec::new(),
      base,
    })
  } else {
    info!(
      judging = partition.stale.len(),
      reusing = partition.reused.len(),
      "sending the paths that have moved, and any still carrying a finding"
    );
    debug!(paths = ?partition.stale, "the paths under review this round");
    let verdict = reviewer::review(
      &request.reviewer,
      &reviewer::packet(&Scope {
        worktree,
        base: &base.commit,
        stale: &partition.stale,
        origins: &changed.origins,
        history: &recorded.history(&key),
      })?,
    )?;
    info!(findings = verdict.findings.len(), "the reviewer returned a verdict");
    recorded.absorb(
      &key,
      worktree.root(),
      &blobs,
      &partition.stale,
      verdict.findings,
    );
    recorded.store(path, &key)?;
    Ok(Outcome::Reviewed {
      standing: recorded.standing(&key),
      reviewed: partition.stale,
      base,
    })
  }
}
