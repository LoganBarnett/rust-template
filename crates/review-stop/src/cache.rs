use crate::error::AppError;
use crate::fingerprint;
use crate::reviewer::Verdict;
use crate::worktree;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

/// The verdict cache entry for the working tree as it stands: content
/// addressed, so the same tree in the same repository at the same `HEAD` maps
/// to the same file whichever session asks.  A tree reviewed clean stays free
/// to end turns on until it changes or `HEAD` moves; a tree reviewed with
/// findings keeps blocking with them, and neither runs the reviewer again.
///
/// The cache lives under the temp directory rather than the repository so it
/// is never in the tree it judges.  It is written by the same user the agent
/// runs as, so a deliberate write can forge it; the gate defends against an
/// agent steering the review through its ordinary tools, not against tampering
/// with the user's own files.
pub struct Entry {
  dir: PathBuf,
  head: Option<String>,
  fingerprint: String,
}

impl Entry {
  pub fn current(files: &[String]) -> Result<Self, AppError> {
    Ok(Self {
      dir: std::env::temp_dir()
        .join("review-stop")
        .join(fingerprint::of_text(&worktree::toplevel()?)?),
      head: worktree::head_commit()?,
      fingerprint: fingerprint::of(files)?,
    })
  }

  /// The commit the conventions and the diff base come from; `None` on an
  /// unborn branch.
  pub fn head(&self) -> Option<&str> {
    self.head.as_deref()
  }

  fn file(&self) -> PathBuf {
    self.dir.join(format!(
      "{}-{}.json",
      self.head.as_deref().unwrap_or("unborn"),
      self.fingerprint
    ))
  }

  /// The verdict recorded for this tree, if one is.  A missing file is the
  /// normal unreviewed case, not a failure.
  pub fn verdict(&self) -> Result<Option<Verdict>, AppError> {
    let path = self.file();
    match fs::read_to_string(&path) {
      Ok(text) => serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| AppError::CacheParse { path, source }),
      Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
      Err(source) => Err(AppError::CacheRead { path, source }),
    }
  }

  pub fn store(&self, verdict: &Verdict) -> Result<(), AppError> {
    fs::create_dir_all(&self.dir).map_err(|source| {
      AppError::CacheDirCreate {
        path: self.dir.clone(),
        source,
      }
    })?;
    let path = self.file();
    serde_json::to_string_pretty(verdict)
      .map_err(AppError::CacheSerialize)
      .and_then(|text| {
        fs::write(&path, text)
          .map_err(|source| AppError::CacheWrite { path, source })
      })
  }
}
