//! One pass of the review: work out what to compare, what still needs
//! looking at, and what stands once the reviewer has spoken.

use crate::base::{self, Base, DiffMode};
use crate::error::ReviewError;
use crate::fingerprint::{self, Unreadable};
use crate::markup::Markup;
use crate::record::{self, Partition, Record, Standing};
use crate::reviewer::{self, Scope, Verdict};
use crate::worktree::{ChangeSet, Worktree};
use gix::ObjectId;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tap::Tap;
use tracing::{debug, info, warn};

/// What to review and how.
#[derive(Debug, Clone)]
pub struct Request {
  /// The comparison to run, or `None` to decide from the tree.
  pub diff: Option<DiffMode>,
  pub reviewer: reviewer::Options,
  /// What to do with the findings recorded from earlier passes.
  pub priors: Priors,
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
    /// The paths this pass could not read, and so judged nothing about.
    skipped: Vec<Skipped>,
    /// What became of the findings recorded before this pass.
    priors: PriorsApplied,
  },
}

/// What a pass did with the findings recorded before it ran, with the count
/// so a front-end can say whether the choice mattered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorsApplied {
  /// The recorded findings were in force going in.
  Kept { recorded: usize },
  /// The recorded findings were forgotten before judging.
  Cleared { forgotten: usize },
}

/// A path the pass could not read.  It is left out of the record, so the next
/// pass tries it again.
#[derive(Debug, Clone)]
pub struct Skipped {
  pub path: String,
  pub reason: String,
}

/// What a pass does with the findings recorded from earlier passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priors {
  /// Carry each one forward until the file it names changes.
  Keep,
  /// Forget them, and judge the files that carried them afresh with no
  /// history to anchor the reviewer.  Clean verdicts on unchanged files are
  /// kept: there is nothing to put to the reviewer again about those.
  Clear,
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
    judgement(request, &worktree, &path, base, &changed)
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
fn judgement(
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
  let hashed = fingerprint::blobs(worktree, &changed.paths)?;
  let key = record_key(&base, request.reviewer.markup);
  let (priors, recorded) =
    priors_applied(request.priors, Record::load(path)?, &key);
  let partition = recorded.partition(&key, &hashed.hashes);
  let judged = if partition.stale.is_empty() {
    info!(
      reused = partition.reused.len(),
      "every changed path was already judged at its current content; \
       no reviewer needed"
    );
    recorded
  } else {
    let history = recorded.history(&key, &hashed.hashes);
    recorded
      .absorb(
        &key,
        worktree.root(),
        &hashed.hashes,
        &partition.stale,
        verdict(
          request,
          worktree,
          &base.commit,
          &partition,
          &changed.origins,
          &history,
        )?
        .findings,
      )
      .store(path, &key)?
  };
  Ok(Outcome::Reviewed {
    standing: judged.standing(&key),
    reviewed: partition.stale,
    skipped: skipped(&hashed.unreadable),
    priors,
    base,
  })
}

/// The record as this pass reads it, and what that did to its findings.
fn priors_applied(
  priors: Priors,
  loaded: Record,
  key: &str,
) -> (PriorsApplied, Record) {
  match priors {
    Priors::Keep => (
      PriorsApplied::Kept {
        recorded: loaded.standing(key).count(),
      },
      loaded,
    ),
    Priors::Clear => {
      let (cleared, forgotten) = loaded.without_priors(key);
      (
        PriorsApplied::Cleared {
          forgotten: forgotten.tap(|forgotten| {
            info!(
              forgotten,
              "forgot the findings recorded for this comparison; the files \
               that carried them are judged afresh"
            );
          }),
        },
        cleared,
      )
    }
  }
}

/// The reviewer's verdict on the paths that are stale this round.
fn verdict(
  request: &Request,
  worktree: &Worktree,
  base: &ObjectId,
  partition: &Partition,
  origins: &BTreeMap<String, String>,
  history: &str,
) -> Result<Verdict, ReviewError> {
  info!(
    judging = partition.stale.len(),
    reusing = partition.reused.len(),
    "sending the paths that have moved, and any still carrying a finding"
  );
  debug!(paths = ?partition.stale, "the paths under review this round");
  reviewer::review(
    &request.reviewer,
    &reviewer::packet(&Scope {
      worktree,
      base,
      stale: &partition.stale,
      origins,
      history,
    })?,
  )
  .inspect(|returned| {
    info!(
      findings = returned.findings.len(),
      "the reviewer returned a verdict"
    );
  })
}

/// Set each unreadable path aside, saying so as it happens.  The warning is
/// the log's account; the outcome carries the same list for the report, which
/// is what a reader sees.
fn skipped(unreadable: &[Unreadable]) -> Vec<Skipped> {
  unreadable
    .iter()
    .inspect(|unread| {
      warn!(
        path = %unread.path,
        error = %unread.source,
        "could not read the path; it is left out of this pass and tried \
         again next time"
      );
    })
    .map(|unread| Skipped {
      path: unread.path.clone(),
      reason: unread.source.to_string(),
    })
    .collect()
}
