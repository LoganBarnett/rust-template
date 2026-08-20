use crate::error::AppError;
use serde::Deserialize;
use std::io;
use std::path::{Path, PathBuf};

/// The document Claude Code writes to a Stop hook's stdin.  Every field is
/// genuinely optional: a harness that predates `permission_mode` omits it, and
/// the gate must run as before when it does.
#[derive(Debug, Deserialize)]
pub struct HookInput {
  session_id: Option<String>,
  transcript_path: Option<PathBuf>,
  permission_mode: Option<String>,
}

impl HookInput {
  pub fn from_stdin() -> Result<Self, AppError> {
    io::read_to_string(io::stdin())
      .map_err(AppError::HookInputRead)
      .and_then(|raw| {
        serde_json::from_str(&raw).map_err(AppError::HookInputParse)
      })
  }

  /// The session the per-session state files are keyed on.
  pub fn session_id(&self) -> &str {
    self.session_id.as_deref().unwrap_or("unknown")
  }

  /// The transcript path, when one was sent and it exists.
  pub fn existing_transcript_path(&self) -> Option<&Path> {
    self
      .transcript_path
      .as_deref()
      .filter(|path| path.is_file())
  }

  pub fn plan_mode(&self) -> bool {
    self.permission_mode.as_deref() == Some("plan")
  }
}
