//! The review record: what has been judged, and what was said about it.
//!
//! A clean verdict is cached; a finding is not.  A finding is a claim about a
//! file that the rest of the tree can invalidate, so any file carrying one is
//! re-reviewed alongside whatever changed — otherwise a finding whose fix
//! belongs in a different file could never clear, because the file it names
//! never moves.
//!
//! The record lives at the repository root under a name a contributor can
//! find, rather than in a temporary directory keyed by a hash.  It is
//! machine-local state about an uncommitted tree: git ignores it, `worktree`
//! filters it out of every listing, and deleting it costs nothing but a
//! re-review.

use crate::error::ReviewError;
use crate::reviewer::Finding;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// The schema this build writes.  A record at any other version is discarded
/// rather than migrated: it caches an uncommitted tree, so one re-review costs
/// less than a migration path per version.
const SCHEMA: u32 = 1;

/// How many rounds of history the record keeps and the packet carries.
const TRANSCRIPT_ROUNDS: usize = 8;

/// How many findings of a single round the history renders before summarising
/// the rest, so one padded round cannot crowd out the changes.
const ROUND_FINDINGS: usize = 20;

/// How many comparisons the record remembers.  Alternating between reviewing
/// the working tree and reviewing the branch is ordinary, and discarding one
/// to make room for the other would re-review everything on each switch.  Two
/// comparisons times three markups is six live combinations, so this leaves a
/// little headroom above that rather than sitting exactly on it.
const BUCKETS: usize = 8;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Record {
  pub schema: u32,
  /// Bumped on every store, so the least recently written bucket is the one
  /// evicted.  A counter rather than a clock: it needs to order writes, not to
  /// date them, and a clock that steps backwards would reorder them.
  pub seq: u64,
  /// One entry per comparison, keyed by `Base::key`.
  pub reviews: BTreeMap<String, Bucket>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Bucket {
  pub seq: u64,
  /// Rounds ever run against this comparison; survives transcript trimming so
  /// the history can say how long this has gone on.
  pub rounds: u32,
  pub files: BTreeMap<String, FileReview>,
  /// The last round's findings against paths outside the change set.  Replaced
  /// wholesale each round, so one survives at most one further round and can
  /// never wedge the review.
  pub unscoped: Vec<Finding>,
  pub transcript: Vec<Round>,
}

/// One path's standing verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReview {
  /// The content hash when the path was judged; `None` for a deletion.  A
  /// state rather than a sentinel string, so nothing can collide with it.
  pub blob: Option<String>,
  pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Round {
  pub round: u32,
  pub reviewed: Vec<String>,
  pub findings: Vec<Finding>,
}

/// How a change set splits for one run.  `stale` is what the packet carries;
/// `reused` contributes its recorded findings unchanged.
#[derive(Debug)]
pub struct Partition {
  pub stale: Vec<String>,
  pub reused: Vec<String>,
}

/// The findings in force for a change set.
#[derive(Debug, Default)]
pub struct Standing {
  pub attributed: Vec<Finding>,
  /// Findings the reviewer reported against paths outside the change set.
  pub unscoped: Vec<Finding>,
}

impl Standing {
  pub fn passes(&self) -> bool {
    self.attributed.is_empty() && self.unscoped.is_empty()
  }

  pub fn count(&self) -> usize {
    self.attributed.len() + self.unscoped.len()
  }
}

/// Probes the version before the whole document, so a schema bump is a
/// rebuild rather than a hard failure.
#[derive(Deserialize)]
struct SchemaProbe {
  schema: u32,
}

impl Record {
  /// The record at `path`, or an empty one when there is nothing usable
  /// there.  A file that is not JSON at all, and one that claims this schema
  /// and then fails to parse, are both errors naming the file: that is
  /// corruption a human should see, not a version this build predates.
  pub fn load(path: &Path) -> Result<Self, ReviewError> {
    match fs::read_to_string(path) {
      Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::empty()),
      Err(source) => Err(ReviewError::RecordRead {
        path: path.to_path_buf(),
        source,
      }),
      Ok(text) => Self::parse(path, &text),
    }
  }

  /// Corruption errors rather than rebuilding quietly, so a human learns
  /// which file to delete instead of wondering why every pass re-reviews
  /// everything.
  fn parse(path: &Path, text: &str) -> Result<Self, ReviewError> {
    let corrupt = |source| ReviewError::RecordParse {
      path: path.to_path_buf(),
      source,
    };
    serde_json::from_str::<serde_json::Value>(text).map_err(corrupt)?;
    match serde_json::from_str::<SchemaProbe>(text) {
      Ok(probe) if probe.schema == SCHEMA => {
        serde_json::from_str(text).map_err(corrupt)
      }
      Ok(probe) => {
        warn!(
          found = probe.schema,
          expected = SCHEMA,
          "the review record is a different schema; starting a fresh one"
        );
        Ok(Self::empty())
      }
      Err(source) => {
        warn!(
          %source,
          "the review record carries no usable schema marker; starting a \
           fresh one"
        );
        Ok(Self::empty())
      }
    }
  }

  fn empty() -> Self {
    Self {
      schema: SCHEMA,
      seq: 0,
      reviews: BTreeMap::new(),
    }
  }

  fn bucket(&self, key: &str) -> Option<&Bucket> {
    self.reviews.get(key)
  }

  /// Which paths need a fresh look.  Nothing is stale when no content has
  /// moved, and that is what keeps an unchanged tree from spending a review.
  pub fn partition(
    &self,
    key: &str,
    blobs: &BTreeMap<String, Option<String>>,
  ) -> Partition {
    let recorded = self.bucket(key);
    let mismatched: BTreeSet<&String> = blobs
      .iter()
      .filter(|(path, blob)| {
        recorded
          .and_then(|bucket| bucket.files.get(path.as_str()))
          .map(|reviewed| &reviewed.blob)
          != Some(blob)
      })
      .map(|(path, _)| path)
      .collect();
    let stale: BTreeSet<&String> = if mismatched.is_empty() {
      BTreeSet::new()
    } else {
      blobs
        .keys()
        .filter(|path| {
          mismatched.contains(path) || self.flagged(recorded, path)
        })
        .collect()
    };
    Partition {
      reused: blobs
        .keys()
        .filter(|path| !stale.contains(path))
        .cloned()
        .collect(),
      stale: stale.into_iter().cloned().collect(),
    }
  }

  fn flagged(&self, recorded: Option<&Bucket>, path: &str) -> bool {
    recorded
      .and_then(|bucket| bucket.files.get(path))
      .is_some_and(|reviewed| !reviewed.findings.is_empty())
  }

  /// The findings in force across the whole change set.
  pub fn standing(&self, key: &str) -> Standing {
    self
      .bucket(key)
      .map_or_else(Standing::default, |bucket| Standing {
        attributed: bucket
          .files
          .values()
          .flat_map(|reviewed| reviewed.findings.iter().cloned())
          .collect(),
        unscoped: bucket.unscoped.clone(),
      })
  }

  /// Fold one round's result into this comparison's bucket.
  pub fn absorb(
    &mut self,
    key: &str,
    root: &Path,
    blobs: &BTreeMap<String, Option<String>>,
    stale: &[String],
    fresh: Vec<Finding>,
  ) {
    let previous = self.bucket(key).cloned().unwrap_or_default();
    let (scoped, unscoped) = attribute(root, blobs, fresh);
    let round = previous.rounds + 1;
    self.seq += 1;
    self.reviews.insert(
      key.to_string(),
      Bucket {
        seq: self.seq,
        rounds: round,
        files: rebuilt(&previous, blobs, stale, &scoped),
        transcript: trimmed(
          previous.transcript,
          Round {
            round,
            reviewed: stale.to_vec(),
            findings: scoped
              .values()
              .flatten()
              .chain(unscoped.iter())
              .cloned()
              .collect(),
          },
        ),
        unscoped,
      },
    );
    self.evict();
  }

  /// Keep only the most recently written comparisons.
  fn evict(&mut self) {
    if self.reviews.len() > BUCKETS {
      let keep: BTreeSet<String> = self
        .reviews
        .iter()
        .map(|(key, bucket)| (bucket.seq, key.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .rev()
        .take(BUCKETS)
        .map(|(_, key)| key)
        .collect();
      self.reviews.retain(|key, _| keep.contains(key));
    }
  }

  /// What the reviewer already said about this comparison, marked against
  /// what still stands, so it does not ask twice.  Empty until a round has
  /// run.
  pub fn history(&self, key: &str) -> String {
    self.bucket(key).map_or_else(String::new, |bucket| {
      if bucket.transcript.is_empty() {
        String::new()
      } else {
        let standing = self.standing(key);
        format!(
          "{}\n{}",
          summary(bucket),
          bucket
            .transcript
            .iter()
            .map(|round| render_round(round, &standing))
            .collect::<String>()
        )
      }
    })
  }

  /// Write the record, replacing only this run's bucket so a concurrent run's
  /// other comparisons survive.  The write is a rename over a temporary file
  /// in the same directory, so a reader never sees a half-written document and
  /// a crash never leaves one.
  pub fn store(&self, path: &Path, key: &str) -> Result<(), ReviewError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let merged = Self::load(path).map(|mut on_disk| {
      if let Some(bucket) = self.bucket(key) {
        on_disk.reviews.insert(key.to_string(), bucket.clone());
      }
      on_disk.schema = SCHEMA;
      on_disk.seq = on_disk.seq.max(self.seq);
      on_disk.evict();
      on_disk
    })?;
    let mut file = tempfile::Builder::new()
      // The temporary file shares the record's name prefix so the same
      // .gitignore entry and the same listing filter cover it.
      .prefix("review.json.")
      .tempfile_in(dir)
      .map_err(|source| ReviewError::RecordTempCreate {
        dir: dir.to_path_buf(),
        source,
      })?;
    let persist = |source| ReviewError::RecordPersist {
      path: path.to_path_buf(),
      source,
    };
    file
      .write_all(
        serde_json::to_string_pretty(&merged)
          .map_err(ReviewError::RecordSerialize)?
          .as_bytes(),
      )
      .map_err(persist)?;
    file
      .persist(path)
      .map(|_| ())
      .map_err(|error| persist(error.error))
  }

  /// Remove the record entirely, for a tree with nothing to review.  An
  /// artifact left in the repository root after the work is done is its own
  /// support question.
  pub fn remove(path: &Path) -> Result<(), ReviewError> {
    match fs::remove_file(path) {
      Ok(()) => {
        debug!("nothing to review; removed the review record");
        Ok(())
      }
      Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
      Err(source) => Err(ReviewError::RecordRemove {
        path: path.to_path_buf(),
        source,
      }),
    }
  }
}

/// This round's findings split into those naming a path under review, grouped
/// by that path, and those naming anything else.  A finding outside the change
/// set is not dropped, but it is kept apart: it is warned about, and the
/// bucket that holds it is replaced wholesale next round.
fn attribute(
  root: &Path,
  blobs: &BTreeMap<String, Option<String>>,
  fresh: Vec<Finding>,
) -> (BTreeMap<String, Vec<Finding>>, Vec<Finding>) {
  let (scoped, unscoped): (Vec<Finding>, Vec<Finding>) = fresh
    .into_iter()
    .map(|finding| normalize(root, finding))
    .partition(|finding| blobs.contains_key(&finding.path));
  unscoped.iter().for_each(|finding| {
    warn!(
      path = %finding.path,
      "the reviewer reported a finding outside the change set"
    );
  });
  (
    scoped.into_iter().fold(
      BTreeMap::<String, Vec<Finding>>::new(),
      |mut grouped, finding| {
        grouped
          .entry(finding.path.clone())
          .or_default()
          .push(finding);
        grouped
      },
    ),
    unscoped,
  )
}

/// Every changed path's entry, built from this round rather than edited into
/// the last one: a stale path the reviewer said nothing about is recorded
/// clean, which is how a file leaves the flagged set, and a path that has left
/// the change set is simply not rebuilt, which is how it is pruned.
fn rebuilt(
  previous: &Bucket,
  blobs: &BTreeMap<String, Option<String>>,
  stale: &[String],
  scoped: &BTreeMap<String, Vec<Finding>>,
) -> BTreeMap<String, FileReview> {
  let judged: BTreeSet<&str> = stale.iter().map(String::as_str).collect();
  blobs
    .iter()
    .map(|(path, blob)| {
      let found = scoped.get(path).cloned().unwrap_or_default();
      (
        path.clone(),
        if judged.contains(path.as_str()) {
          FileReview {
            blob: blob.clone(),
            findings: found,
          }
        } else {
          reused(previous.files.get(path), blob, found)
        },
      )
    })
    .collect()
}

/// A path this round did not judge keeps what it was judged on.  It can still
/// pick up a finding the reviewer volunteered about it, which flags it so the
/// next round judges it directly.
fn reused(
  kept: Option<&FileReview>,
  blob: &Option<String>,
  found: Vec<Finding>,
) -> FileReview {
  FileReview {
    blob: kept.map_or_else(|| blob.clone(), |entry| entry.blob.clone()),
    findings: kept
      .map(|entry| entry.findings.clone())
      .unwrap_or_default()
      .into_iter()
      .chain(found)
      .collect(),
  }
}

/// The transcript with this round appended, holding the most recent rounds.
/// Trimming from the end keeps the newest, which is what the next packet wants
/// to show the reviewer.
fn trimmed(previous: Vec<Round>, latest: Round) -> Vec<Round> {
  previous
    .into_iter()
    .chain(std::iter::once(latest))
    .rev()
    .take(TRANSCRIPT_ROUNDS)
    .collect::<Vec<_>>()
    .into_iter()
    .rev()
    .collect()
}

/// The record's path for a repository root.
pub fn path_for(root: &Path) -> PathBuf {
  root.join(crate::worktree::RECORD_PREFIX)
}

fn summary(bucket: &Bucket) -> String {
  let omitted = bucket.rounds as usize - bucket.transcript.len();
  let note = if omitted > 0 {
    format!("  ({omitted} earlier round(s) omitted.)")
  } else {
    String::new()
  };
  format!("Round {} of this review so far.{note}\n", bucket.rounds)
}

fn render_round(round: &Round, standing: &Standing) -> String {
  let shown = round
    .findings
    .iter()
    .take(ROUND_FINDINGS)
    .map(|finding| {
      format!(
        "  [{}] {}:{}  {} ({})\n        fix: {}\n",
        if still_stands(finding, standing) {
          "still stands"
        } else {
          "addressed"
        },
        finding.path,
        finding.line,
        finding.convention,
        finding.document,
        finding.fix,
      )
    })
    .collect::<String>();
  let more = round.findings.len().saturating_sub(ROUND_FINDINGS);
  let tail = if more > 0 {
    format!("  … and {more} more findings this round.\n")
  } else {
    String::new()
  };
  format!(
    "\nRound {} — reviewed {}\n{}{}",
    round.round,
    round.reviewed.join(", "),
    if shown.is_empty() {
      "  (nothing reported)\n".to_string()
    } else {
      shown
    },
    tail,
  )
}

/// Whether a historical finding is still in force.  Matched on path and
/// convention only: the line drifts as the file is edited, and the fix is
/// prose the reviewer regenerates, so neither identifies a finding across
/// rounds.  This is a heuristic, and a reviewer that rewords a convention
/// phrase will read as having raised something new.
fn still_stands(finding: &Finding, standing: &Standing) -> bool {
  standing
    .attributed
    .iter()
    .chain(standing.unscoped.iter())
    .any(|current| {
      current.path == finding.path && current.convention == finding.convention
    })
}

/// A finding's path as the change set spells it.  A reviewer may answer with a
/// leading `./` or an absolute path; without this every such finding would
/// land outside the change set and the per-file record would quietly stop
/// working.
fn normalize(root: &Path, finding: Finding) -> Finding {
  let trimmed = finding.path.strip_prefix("./").unwrap_or(&finding.path);
  let path = Path::new(trimmed).strip_prefix(root).map_or_else(
    |_| trimmed.to_string(),
    |relative| relative.to_string_lossy().into_owned(),
  );
  Finding { path, ..finding }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn finding(path: &str, convention: &str) -> Finding {
    Finding {
      path: path.to_string(),
      line: 1,
      convention: convention.to_string(),
      document: "llms.org".to_string(),
      fix: "do the thing".to_string(),
    }
  }

  fn blobs(
    entries: &[(&str, Option<&str>)],
  ) -> BTreeMap<String, Option<String>> {
    entries
      .iter()
      .map(|(path, blob)| ((*path).to_string(), blob.map(str::to_string)))
      .collect()
  }

  #[test]
  fn a_record_from_another_schema_rebuilds_quietly() {
    let parsed = Record::parse(Path::new("review.json"), r#"{"schema":99}"#);
    assert!(
      parsed.is_ok_and(|record| record.reviews.is_empty()),
      "a version this build predates is not an error",
    );
  }

  #[test]
  fn a_record_that_is_not_json_names_itself() {
    assert!(
      matches!(
        Record::parse(Path::new("review.json"), r#"{"schema":1,"revi"#),
        Err(ReviewError::RecordParse { .. }),
      ),
      "corruption is reported, not silently discarded",
    );
  }

  #[test]
  fn an_unchanged_tree_is_never_stale() {
    let mut record = Record::empty();
    let content = blobs(&[("a.rs", Some("aaa"))]);
    record.absorb(
      "head:x",
      Path::new("/r"),
      &content,
      &["a.rs".to_string()],
      vec![],
    );
    assert!(record.partition("head:x", &content).stale.is_empty());
  }

  #[test]
  fn a_flagged_file_rides_along_with_whatever_changed() {
    let mut record = Record::empty();
    let first = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("bbb"))]);
    record.absorb(
      "head:x",
      Path::new("/r"),
      &first,
      &["a.rs".to_string(), "b.rs".to_string()],
      vec![finding("a.rs", "comments are sentences")],
    );
    // Only b.rs moved, but a.rs carries a finding whose fix may well live in
    // b.rs, so it must be judged again too.
    let second = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("CHANGED"))]);
    let partition = record.partition("head:x", &second);
    assert_eq!(partition.stale, vec!["a.rs".to_string(), "b.rs".to_string()],);
  }

  #[test]
  fn only_the_changed_file_is_stale_when_nothing_is_flagged() {
    let mut record = Record::empty();
    let first = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("bbb"))]);
    record.absorb(
      "head:x",
      Path::new("/r"),
      &first,
      &["a.rs".to_string(), "b.rs".to_string()],
      vec![],
    );
    let second = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("CHANGED"))]);
    assert_eq!(
      record.partition("head:x", &second).stale,
      vec!["b.rs".to_string()],
    );
  }

  #[test]
  fn addressing_a_file_clears_its_findings() {
    let mut record = Record::empty();
    let first = blobs(&[("a.rs", Some("aaa"))]);
    record.absorb(
      "head:x",
      Path::new("/r"),
      &first,
      &["a.rs".to_string()],
      vec![finding("a.rs", "comments are sentences")],
    );
    assert_eq!(record.standing("head:x").count(), 1);
    let second = blobs(&[("a.rs", Some("FIXED"))]);
    record.absorb(
      "head:x",
      Path::new("/r"),
      &second,
      &["a.rs".to_string()],
      vec![],
    );
    assert!(record.standing("head:x").passes());
  }

  #[test]
  fn a_path_that_leaves_the_change_set_is_pruned() {
    let mut record = Record::empty();
    record.absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("aaa"))]),
      &["a.rs".to_string()],
      vec![finding("a.rs", "comments are sentences")],
    );
    record.absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("b.rs", Some("bbb"))]),
      &["b.rs".to_string()],
      vec![],
    );
    assert!(record.standing("head:x").passes());
  }

  #[test]
  fn a_finding_outside_the_change_set_is_kept_apart() {
    let mut record = Record::empty();
    record.absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("aaa"))]),
      &["a.rs".to_string()],
      vec![finding("elsewhere.rs", "unrelated")],
    );
    let standing = record.standing("head:x");
    assert!(standing.attributed.is_empty());
    assert_eq!(standing.unscoped.len(), 1);
  }

  #[test]
  fn an_out_of_scope_finding_retires_after_one_round() {
    let mut record = Record::empty();
    record.absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("aaa"))]),
      &["a.rs".to_string()],
      vec![finding("elsewhere.rs", "unrelated")],
    );
    record.absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("CHANGED"))]),
      &["a.rs".to_string()],
      vec![],
    );
    assert!(record.standing("head:x").passes());
  }

  #[test]
  fn a_finding_path_is_normalized_against_the_root() {
    let mut record = Record::empty();
    record.absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("aaa"))]),
      &["a.rs".to_string()],
      vec![finding("/r/a.rs", "absolute"), finding("./a.rs", "dotted")],
    );
    let standing = record.standing("head:x");
    assert_eq!(
      standing.attributed.len(),
      2,
      "both spellings name a path in the change set",
    );
    assert!(standing.unscoped.is_empty());
  }

  #[test]
  fn the_transcript_is_bounded_and_says_what_it_dropped() {
    let mut record = Record::empty();
    (0..TRANSCRIPT_ROUNDS + 2).for_each(|round| {
      record.absorb(
        "head:x",
        Path::new("/r"),
        &blobs(&[("a.rs", Some(&format!("blob{round}")))]),
        &["a.rs".to_string()],
        vec![finding("a.rs", "comments are sentences")],
      );
    });
    let history = record.history("head:x");
    assert!(history.contains("2 earlier round(s) omitted"));
    assert_eq!(
      history.matches("Round ").count(),
      TRANSCRIPT_ROUNDS + 1,
      "the summary line plus one heading per retained round",
    );
  }

  #[test]
  fn history_marks_what_was_addressed() {
    let mut record = Record::empty();
    record.absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("aaa"))]),
      &["a.rs".to_string()],
      vec![finding("a.rs", "comments are sentences")],
    );
    record.absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("FIXED"))]),
      &["a.rs".to_string()],
      vec![],
    );
    assert!(record.history("head:x").contains("[addressed]"));
  }

  #[test]
  fn buckets_do_not_share_verdicts() {
    let mut record = Record::empty();
    let content = blobs(&[("a.rs", Some("aaa"))]);
    record.absorb(
      "head:x",
      Path::new("/r"),
      &content,
      &["a.rs".to_string()],
      vec![],
    );
    assert!(
      !record.partition("branch:y", &content).stale.is_empty(),
      "a head verdict must not satisfy a branch request",
    );
  }

  #[test]
  fn only_the_most_recent_comparisons_survive() {
    let mut record = Record::empty();
    let content = blobs(&[("a.rs", Some("aaa"))]);
    (0..BUCKETS + 2).for_each(|n| {
      record.absorb(
        &format!("head:{n}"),
        Path::new("/r"),
        &content,
        &["a.rs".to_string()],
        vec![],
      );
    });
    assert_eq!(record.reviews.len(), BUCKETS);
    assert!(record
      .reviews
      .contains_key(&format!("head:{}", BUCKETS + 1)));
    assert!(!record.reviews.contains_key("head:0"));
  }
}
