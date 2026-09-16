//! The review record: what has been judged, and what was said about it.
//!
//! A verdict is cached either way.  A clean one holds until the file moves; a
//! finding stands against the file it names until that file moves or the
//! caller clears the priors, whatever the reviewer says or does not say about
//! it in between.  A file carrying one still rides along into the next round,
//! so the change is judged in the context of what stands against it and the
//! reviewer can add to it, but its silence about a finding it was told not to
//! repeat takes nothing away.
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

  /// The record with every finding recorded for `key` forgotten, along with
  /// the rounds that produced them, and how many were forgotten.  The clean
  /// verdicts stay, so an unchanged clean file is still reused; a file that
  /// carried a finding is no longer recorded at all, which is what puts it
  /// back in front of the reviewer with nothing to anchor it.
  pub fn without_priors(self, key: &str) -> (Self, usize) {
    let forgotten = self.standing(key).count();
    (
      Self {
        schema: self.schema,
        seq: self.seq,
        reviews: self
          .reviews
          .into_iter()
          .map(|(name, bucket)| {
            if name == key {
              (name, without_findings(bucket))
            } else {
              (name, bucket)
            }
          })
          .collect(),
      },
      forgotten,
    )
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

  /// The record with one round's result folded into this comparison's
  /// bucket.
  pub fn absorb(
    self,
    key: &str,
    root: &Path,
    blobs: &BTreeMap<String, Option<String>>,
    stale: &[String],
    fresh: Vec<Finding>,
  ) -> Self {
    let previous = self.bucket(key).cloned().unwrap_or_default();
    let (scoped, unscoped) = attribute(root, blobs, fresh);
    let round = previous.rounds + 1;
    let seq = self.seq + 1;
    Self {
      schema: self.schema,
      seq,
      reviews: self
        .reviews
        .into_iter()
        .chain(std::iter::once((
          key.to_string(),
          Bucket {
            seq,
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
        )))
        .collect(),
    }
    .evicted()
  }

  /// The record holding only the most recently written comparisons.
  fn evicted(self) -> Self {
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
    Self {
      schema: self.schema,
      seq: self.seq,
      reviews: self
        .reviews
        .into_iter()
        .filter(|(key, _)| keep.contains(key))
        .collect(),
    }
  }

  /// What the reviewer already said about this comparison, marked against
  /// what still stands and whether each file has moved since, so it does not
  /// ask twice.  Empty until a round has run.
  pub fn history(
    &self,
    key: &str,
    blobs: &BTreeMap<String, Option<String>>,
  ) -> String {
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
            .map(|round| render_round(round, &standing, bucket, blobs))
            .collect::<String>()
        )
      }
    })
  }

  /// The record, written so that only this run's bucket replaces what is on
  /// disk and a concurrent run's other comparisons survive.  The write is a
  /// rename over a temporary file in the same directory, so a reader never
  /// sees a half-written document and a crash never leaves one.
  pub fn store(self, path: &Path, key: &str) -> Result<Self, ReviewError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let merged = Self::load(path).map(|on_disk| {
      Self {
        schema: SCHEMA,
        seq: on_disk.seq.max(self.seq),
        reviews: on_disk
          .reviews
          .into_iter()
          .chain(
            self
              .bucket(key)
              .cloned()
              .map(|bucket| (key.to_string(), bucket)),
          )
          .collect(),
      }
      .evicted()
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
      .map(|_| self)
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
/// the last one.  A judged path that moved and that the reviewer said nothing
/// about is recorded clean, which is how a file leaves the flagged set; one
/// that has not moved keeps what stood against it, since the reviewer is told
/// not to repeat those and its silence is not a verdict.  A path that has left
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
            findings: judged_findings(previous.files.get(path), blob, found),
          }
        } else {
          reused(previous.files.get(path), blob, found)
        },
      )
    })
    .collect()
}

/// The bucket with its clean verdicts and nothing else.
fn without_findings(bucket: Bucket) -> Bucket {
  Bucket {
    seq: bucket.seq,
    files: bucket
      .files
      .into_iter()
      .filter(|(_, review)| review.findings.is_empty())
      .collect(),
    ..Bucket::default()
  }
}

/// A judged path's findings: what this round reported, on top of what already
/// stood when the path has not moved since it was last judged.  A finding the
/// reviewer repeats regardless is kept once.
fn judged_findings(
  kept: Option<&FileReview>,
  blob: &Option<String>,
  found: Vec<Finding>,
) -> Vec<Finding> {
  let carried: Vec<Finding> = kept
    .filter(|entry| entry.blob == *blob)
    .map(|entry| entry.findings.clone())
    .unwrap_or_default();
  let fresh: Vec<Finding> = found
    .into_iter()
    .filter(|finding| !carried.iter().any(|stood| same(stood, finding)))
    .collect();
  carried.into_iter().chain(fresh).collect()
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

fn render_round(
  round: &Round,
  standing: &Standing,
  bucket: &Bucket,
  blobs: &BTreeMap<String, Option<String>>,
) -> String {
  let shown = round
    .findings
    .iter()
    .take(ROUND_FINDINGS)
    .map(|finding| {
      format!(
        "  [{}] {}:{}  {} ({})\n        fix: {}\n",
        mark(finding, standing, bucket, blobs),
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
    .any(|current| same(current, finding))
}

/// Whether two findings are the same one across rounds; see `still_stands`
/// for why this is the comparison.
fn same(a: &Finding, b: &Finding) -> bool {
  a.path == b.path && a.convention == b.convention
}

/// How the reviewer should treat a historical finding: `addressed` once it no
/// longer stands, `re-judge` while its file has moved since it was judged, and
/// `carried forward` while the file has not, in which case the record keeps
/// the finding without the reviewer's help.
fn mark(
  finding: &Finding,
  standing: &Standing,
  bucket: &Bucket,
  blobs: &BTreeMap<String, Option<String>>,
) -> &'static str {
  if !still_stands(finding, standing) {
    "addressed"
  } else if moved(bucket, blobs, &finding.path) {
    "re-judge"
  } else {
    "carried forward"
  }
}

/// Whether a path's content differs from what it was last judged on.  A path
/// the record never judged, or one no longer in the change set, counts as
/// moved: neither has a verdict to carry.
fn moved(
  bucket: &Bucket,
  blobs: &BTreeMap<String, Option<String>>,
  path: &str,
) -> bool {
  bucket
    .files
    .get(path)
    .zip(blobs.get(path))
    .is_none_or(|(kept, blob)| kept.blob != *blob)
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
      parsed.is_ok_and(|rebuilt| rebuilt.reviews.is_empty()),
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
    let content = blobs(&[("a.rs", Some("aaa"))]);
    let judged = Record::empty().absorb(
      "head:x",
      Path::new("/r"),
      &content,
      &["a.rs".to_string()],
      vec![],
    );
    assert!(judged.partition("head:x", &content).stale.is_empty());
  }

  #[test]
  fn a_flagged_file_rides_along_with_whatever_changed() {
    let first = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("bbb"))]);
    let flagged = Record::empty().absorb(
      "head:x",
      Path::new("/r"),
      &first,
      &["a.rs".to_string(), "b.rs".to_string()],
      vec![finding("a.rs", "comments are sentences")],
    );
    // Only b.rs moved, but a.rs carries a finding whose fix may well live in
    // b.rs, so it must be judged again too.
    let second = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("CHANGED"))]);
    let partition = flagged.partition("head:x", &second);
    assert_eq!(partition.stale, vec!["a.rs".to_string(), "b.rs".to_string()],);
  }

  #[test]
  fn only_the_changed_file_is_stale_when_nothing_is_flagged() {
    let first = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("bbb"))]);
    let clean = Record::empty().absorb(
      "head:x",
      Path::new("/r"),
      &first,
      &["a.rs".to_string(), "b.rs".to_string()],
      vec![],
    );
    let second = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("CHANGED"))]);
    assert_eq!(
      clean.partition("head:x", &second).stale,
      vec!["b.rs".to_string()],
    );
  }

  #[test]
  fn addressing_a_file_clears_its_findings() {
    let flagged = Record::empty().absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("aaa"))]),
      &["a.rs".to_string()],
      vec![finding("a.rs", "comments are sentences")],
    );
    assert_eq!(flagged.standing("head:x").count(), 1);
    let fixed = flagged.absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("FIXED"))]),
      &["a.rs".to_string()],
      vec![],
    );
    assert!(fixed.standing("head:x").passes());
  }

  #[test]
  fn a_path_that_leaves_the_change_set_is_pruned() {
    let pruned = Record::empty()
      .absorb(
        "head:x",
        Path::new("/r"),
        &blobs(&[("a.rs", Some("aaa"))]),
        &["a.rs".to_string()],
        vec![finding("a.rs", "comments are sentences")],
      )
      .absorb(
        "head:x",
        Path::new("/r"),
        &blobs(&[("b.rs", Some("bbb"))]),
        &["b.rs".to_string()],
        vec![],
      );
    assert!(pruned.standing("head:x").passes());
  }

  #[test]
  fn a_finding_outside_the_change_set_is_kept_apart() {
    let judged = Record::empty().absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("aaa"))]),
      &["a.rs".to_string()],
      vec![finding("elsewhere.rs", "unrelated")],
    );
    let standing = judged.standing("head:x");
    assert!(standing.attributed.is_empty());
    assert_eq!(standing.unscoped.len(), 1);
  }

  #[test]
  fn an_out_of_scope_finding_retires_after_one_round() {
    let retired = Record::empty()
      .absorb(
        "head:x",
        Path::new("/r"),
        &blobs(&[("a.rs", Some("aaa"))]),
        &["a.rs".to_string()],
        vec![finding("elsewhere.rs", "unrelated")],
      )
      .absorb(
        "head:x",
        Path::new("/r"),
        &blobs(&[("a.rs", Some("CHANGED"))]),
        &["a.rs".to_string()],
        vec![],
      );
    assert!(retired.standing("head:x").passes());
  }

  #[test]
  fn a_finding_path_is_normalized_against_the_root() {
    let judged = Record::empty().absorb(
      "head:x",
      Path::new("/r"),
      &blobs(&[("a.rs", Some("aaa"))]),
      &["a.rs".to_string()],
      vec![finding("/r/a.rs", "absolute"), finding("./a.rs", "dotted")],
    );
    let standing = judged.standing("head:x");
    assert_eq!(
      standing.attributed.len(),
      2,
      "both spellings name a path in the change set",
    );
    assert!(standing.unscoped.is_empty());
  }

  #[test]
  fn the_transcript_is_bounded_and_says_what_it_dropped() {
    let long_running =
      (0..TRANSCRIPT_ROUNDS + 2).fold(Record::empty(), |so_far, round| {
        so_far.absorb(
          "head:x",
          Path::new("/r"),
          &blobs(&[("a.rs", Some(&format!("blob{round}")))]),
          &["a.rs".to_string()],
          vec![finding("a.rs", "comments are sentences")],
        )
      });
    let history =
      long_running.history("head:x", &blobs(&[("a.rs", Some("blob0"))]));
    assert!(history.contains("2 earlier round(s) omitted"));
    assert_eq!(
      history.matches("Round ").count(),
      TRANSCRIPT_ROUNDS + 1,
      "the summary line plus one heading per retained round",
    );
  }

  #[test]
  fn history_marks_what_was_addressed() {
    let fixed = Record::empty()
      .absorb(
        "head:x",
        Path::new("/r"),
        &blobs(&[("a.rs", Some("aaa"))]),
        &["a.rs".to_string()],
        vec![finding("a.rs", "comments are sentences")],
      )
      .absorb(
        "head:x",
        Path::new("/r"),
        &blobs(&[("a.rs", Some("FIXED"))]),
        &["a.rs".to_string()],
        vec![],
      );
    assert!(fixed
      .history("head:x", &blobs(&[("a.rs", Some("FIXED"))]))
      .contains("[addressed]"));
  }

  #[test]
  fn a_finding_on_an_unmoved_file_survives_the_reviewers_silence() {
    let first = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("bbb"))]);
    // b.rs moved, a.rs rode along flagged, and the reviewer, told not to
    // repeat what is carried forward, said nothing.
    let second = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("CHANGED"))]);
    let carried = Record::empty()
      .absorb(
        "head:x",
        Path::new("/r"),
        &first,
        &["a.rs".to_string(), "b.rs".to_string()],
        vec![finding("a.rs", "comments are sentences")],
      )
      .absorb(
        "head:x",
        Path::new("/r"),
        &second,
        &["a.rs".to_string(), "b.rs".to_string()],
        vec![],
      );
    assert_eq!(carried.standing("head:x").count(), 1);
  }

  #[test]
  fn a_repeated_finding_is_kept_once() {
    let first = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("bbb"))]);
    let second = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("CHANGED"))]);
    let repeated = Record::empty()
      .absorb(
        "head:x",
        Path::new("/r"),
        &first,
        &["a.rs".to_string(), "b.rs".to_string()],
        vec![finding("a.rs", "comments are sentences")],
      )
      .absorb(
        "head:x",
        Path::new("/r"),
        &second,
        &["a.rs".to_string(), "b.rs".to_string()],
        vec![finding("a.rs", "comments are sentences")],
      );
    assert_eq!(repeated.standing("head:x").count(), 1);
  }

  #[test]
  fn history_says_which_findings_to_re_judge() {
    let first = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("bbb"))]);
    let flagged = Record::empty().absorb(
      "head:x",
      Path::new("/r"),
      &first,
      &["a.rs".to_string(), "b.rs".to_string()],
      vec![
        finding("a.rs", "comments are sentences"),
        finding("b.rs", "no unwrap"),
      ],
    );
    let second = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("CHANGED"))]);
    let history = flagged.history("head:x", &second);
    assert!(history.contains("[carried forward] a.rs"), "{history}");
    assert!(history.contains("[re-judge] b.rs"), "{history}");
  }

  #[test]
  fn clearing_priors_forgets_findings_but_keeps_clean_verdicts() {
    let content = blobs(&[("a.rs", Some("aaa")), ("b.rs", Some("bbb"))]);
    let flagged = Record::empty().absorb(
      "head:x",
      Path::new("/r"),
      &content,
      &["a.rs".to_string(), "b.rs".to_string()],
      vec![finding("a.rs", "comments are sentences")],
    );
    let (cleared, forgotten) = flagged.without_priors("head:x");
    assert_eq!(forgotten, 1);
    assert!(cleared.standing("head:x").passes());
    assert!(cleared.history("head:x", &content).is_empty());
    let partition = cleared.partition("head:x", &content);
    assert_eq!(partition.stale, vec!["a.rs".to_string()]);
    assert_eq!(partition.reused, vec!["b.rs".to_string()]);
  }

  #[test]
  fn buckets_do_not_share_verdicts() {
    let content = blobs(&[("a.rs", Some("aaa"))]);
    let head_judged = Record::empty().absorb(
      "head:x",
      Path::new("/r"),
      &content,
      &["a.rs".to_string()],
      vec![],
    );
    assert!(
      !head_judged.partition("branch:y", &content).stale.is_empty(),
      "a head verdict must not satisfy a branch request",
    );
  }

  #[test]
  fn only_the_most_recent_comparisons_survive() {
    let content = blobs(&[("a.rs", Some("aaa"))]);
    let crowded = (0..BUCKETS + 2).fold(Record::empty(), |so_far, n| {
      so_far.absorb(
        &format!("head:{n}"),
        Path::new("/r"),
        &content,
        &["a.rs".to_string()],
        vec![],
      )
    });
    assert_eq!(crowded.reviews.len(), BUCKETS);
    assert!(crowded
      .reviews
      .contains_key(&format!("head:{}", BUCKETS + 1)));
    assert!(!crowded.reviews.contains_key("head:0"));
  }
}
