//! Configuration file discovery and loading.
//!
//! Provides the three-stage config search (explicit path → `./config.toml` →
//! `$XDG_CONFIG_HOME/<app>/config.toml`) and a generic TOML loader that
//! produces semantic errors.

use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use thiserror::Error;

// ── errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ConfigFileError {
  #[error(
    "Failed to read configuration file at {path:?} during startup: {source}"
  )]
  FileRead {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },

  #[error("Failed to parse configuration file at {path:?}: {source}")]
  Parse {
    path: PathBuf,
    #[source]
    source: toml::de::Error,
  },
}

// ── discovery ───────────────────────────────────────────────────────────────

/// Resolve `$XDG_CONFIG_HOME/<app_name>`, falling back to
/// `$HOME/.config/<app_name>` when the variable is unset.
pub fn xdg_config_dir(app_name: &str) -> Option<PathBuf> {
  std::env::var_os("XDG_CONFIG_HOME")
    .map(PathBuf::from)
    .or_else(|| home::home_dir().map(|h| h.join(".config")))
    .map(|d| d.join(app_name))
}

/// Locate a configuration file using a two-stage search:
///
/// 1. If `explicit_path` is `Some`, return it unconditionally.
/// 2. Fall back to `$XDG_CONFIG_HOME/<app_name>/config.toml`.
///
/// Returns `None` when no candidate exists on disk.
///
/// A working-directory lookup is deliberately omitted.  `config.toml` is a
/// common enough filename that silently picking one up from whatever
/// directory the user happens to be in is a footgun — different config
/// would load depending on cwd, with no warning.  Callers who want a
/// local config can pass its path via `explicit_path` (the `--config`
/// flag / `CONFIG_FILE` env var on [`CommonCli`]).
pub fn find_config_file(
  app_name: &str,
  explicit_path: Option<&Path>,
) -> Option<PathBuf> {
  if let Some(p) = explicit_path {
    return Some(p.to_path_buf());
  }

  xdg_config_dir(app_name)
    .map(|d| d.join("config.toml"))
    .filter(|p| p.exists())
}

/// Deserialise a TOML file into `T`, wrapping I/O and parse failures in
/// [`ConfigFileError`].
pub fn load_toml<T: DeserializeOwned>(
  path: &Path,
) -> Result<T, ConfigFileError> {
  let contents = std::fs::read_to_string(path).map_err(|source| {
    ConfigFileError::FileRead {
      path: path.to_path_buf(),
      source,
    }
  })?;

  toml::from_str(&contents).map_err(|source| ConfigFileError::Parse {
    path: path.to_path_buf(),
    source,
  })
}

// ── CLI / config-file fragment structs ──────────────────────────────────────

/// Common CLI arguments shared by every project crate.  Flatten into your
/// crate's `CliRaw` with `#[command(flatten)]`.
#[cfg(feature = "cli")]
#[derive(Debug, clap::Args)]
pub struct CommonCli {
  /// Log level (trace, debug, info, warn, error).
  #[arg(long, env = "LOG_LEVEL")]
  pub log_level: Option<String>,

  /// Log format (text, json).
  #[arg(long, env = "LOG_FORMAT")]
  pub log_format: Option<String>,

  /// Path to configuration file.
  #[arg(short, long, env = "CONFIG_FILE")]
  pub config: Option<PathBuf>,
}

/// Common config-file fields shared by every project crate.  Flatten into
/// your `ConfigFileRaw` with `#[serde(flatten)]`.
#[derive(Debug, serde::Deserialize, Default)]
pub struct CommonConfigFile {
  pub log_level: Option<String>,
  pub log_format: Option<String>,
}

/// Returns the path to the `oidc-client-secret` credential file inside
/// systemd's `CREDENTIALS_DIRECTORY`, if the directory is set and the
/// file exists.
#[cfg(feature = "server")]
pub fn credential_secret_path() -> Option<PathBuf> {
  let dir = std::env::var("CREDENTIALS_DIRECTORY").ok()?;
  let path = PathBuf::from(dir).join("oidc-client-secret");
  path.exists().then_some(path)
}

/// Resolve `log_level` and `log_format` from CLI → config-file → defaults.
///
/// Returns `(LogLevel, LogFormat)` or an error message suitable for user
/// display.
pub fn resolve_log_settings(
  cli_level: Option<String>,
  cli_format: Option<String>,
  file: &CommonConfigFile,
) -> Result<(crate::logging::LogLevel, crate::logging::LogFormat), String> {
  let level_str = cli_level
    .or_else(|| file.log_level.clone())
    .unwrap_or_else(|| "info".to_string());

  let level = level_str
    .parse::<crate::logging::LogLevel>()
    .map_err(|e| e.to_string())?;

  let format_str = cli_format
    .or_else(|| file.log_format.clone())
    .unwrap_or_else(|| "text".to_string());

  let format = format_str
    .parse::<crate::logging::LogFormat>()
    .map_err(|e| e.to_string())?;

  Ok((level, format))
}
