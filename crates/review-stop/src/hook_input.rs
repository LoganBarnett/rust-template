use crate::error::AppError;
use serde::Deserialize;
use std::io;

/// The document Claude Code writes to a Stop hook's stdin.  Only the permission
/// mode is read; it is genuinely optional, since a harness that predates the
/// field omits it, and the gate must run as before when it does.
#[derive(Debug, Deserialize)]
pub struct HookInput {
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

  pub fn plan_mode(&self) -> bool {
    self.permission_mode.as_deref() == Some("plan")
  }
}
