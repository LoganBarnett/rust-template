use crate::decision::Decision;
use crate::error::AppError;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Consecutive blocks allowed within one turn before the gate gives up.  The
/// first block is usually just "you have not run the reviewer yet", so this
/// leaves a few rounds for actually resolving findings.
pub const MAX_ROUNDS: u32 = 4;

/// The gate's per-session state, kept as files under the temp directory
/// (`TMPDIR` when set) and keyed by session id.  The review round counter is
/// keyed to the turn it was counted in, so a new prompt starts a fresh budget.
pub struct SessionState {
  review_rounds: PathBuf,
  clippy_rounds: PathBuf,
  reviewed: PathBuf,
}

impl SessionState {
  pub fn for_session(session_id: &str) -> Self {
    let dir = std::env::temp_dir();
    Self {
      review_rounds: dir.join(format!("review-stop.{session_id}.state")),
      clippy_rounds: dir.join(format!("review-stop-clippy.{session_id}.state")),
      reviewed: dir.join(format!("review-stop.{session_id}.reviewed")),
    }
  }

  /// A release after a verdict (or the give-up) records the fingerprint it
  /// released on, so a later Stop that finds the same content need not review
  /// it again.
  pub fn settle(&self, decision: &Decision) -> Result<(), AppError> {
    match decision {
      Decision::Release { reviewed } => {
        reviewed
          .as_deref()
          .map(|fingerprint| self.record_reviewed(fingerprint))
          .transpose()?;
        self.remove(&self.review_rounds)
      }
      Decision::Block { .. } => Ok(()),
    }
  }

  /// The fingerprint the gate last released on, if any.
  pub fn reviewed_fingerprint(&self) -> Result<Option<String>, AppError> {
    self
      .read(&self.reviewed)
      .map(|contents| contents.map(|text| text.trim_end().to_string()))
  }

  fn record_reviewed(&self, fingerprint: &str) -> Result<(), AppError> {
    self.write(&self.reviewed, &format!("{fingerprint}\n"))
  }

  /// The review blocks already issued this turn.  A counter recorded for a
  /// different turn — or none, or an unreadable one — counts as zero.
  pub fn review_rounds_after(&self, turn_key: &str) -> Result<u32, AppError> {
    self.read(&self.review_rounds).map(|contents| {
      contents
        .as_deref()
        .and_then(|text| text.split_once(char::is_whitespace))
        .filter(|(key, _)| *key == turn_key)
        .map_or(0, |(_, count)| count.trim().parse().unwrap_or(0))
    })
  }

  pub fn write_review_rounds(
    &self,
    turn_key: &str,
    count: u32,
  ) -> Result<(), AppError> {
    self.write(&self.review_rounds, &format!("{turn_key} {count}\n"))
  }

  /// The failing clippy runs already blocked on this session; missing or
  /// unreadable counts as zero.
  pub fn clippy_rounds(&self) -> Result<u32, AppError> {
    self.read(&self.clippy_rounds).map(|contents| {
      contents.map_or(0, |text| text.trim().parse().unwrap_or(0))
    })
  }

  pub fn write_clippy_rounds(&self, count: u32) -> Result<(), AppError> {
    self.write(&self.clippy_rounds, &format!("{count}\n"))
  }

  pub fn clear_clippy_rounds(&self) -> Result<(), AppError> {
    self.remove(&self.clippy_rounds)
  }

  /// The file's contents, or `None` when it does not exist — a missing state
  /// file is the normal fresh-session case, not a failure.
  fn read(&self, path: &Path) -> Result<Option<String>, AppError> {
    match fs::read_to_string(path) {
      Ok(contents) => Ok(Some(contents)),
      Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
      Err(source) => Err(AppError::StateFileRead {
        path: path.to_path_buf(),
        source,
      }),
    }
  }

  fn write(&self, path: &Path, contents: &str) -> Result<(), AppError> {
    fs::write(path, contents).map_err(|source| AppError::StateFileWrite {
      path: path.to_path_buf(),
      source,
    })
  }

  /// Remove the file; one that is already gone is the desired end state.
  fn remove(&self, path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
      Ok(()) => Ok(()),
      Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
      Err(source) => Err(AppError::StateFileRemove {
        path: path.to_path_buf(),
        source,
      }),
    }
  }
}
