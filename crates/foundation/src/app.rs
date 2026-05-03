//! Application traits for the foundation entry-point macro.
//!
//! `CliApp` is available with the `cli` feature and covers all apps.
//! `ServerApp` (in `server::runner`) extends it for server apps and
//! requires the `auth` feature.

use crate::logging::{LogFormat, LogLevel};

/// Base trait for all foundation-managed apps.
///
/// Implement this on your `Config` type to participate in the standard
/// CLI-parse → config-resolve → logging-init lifecycle.
pub trait CliApp: Sized {
  type CliArgs: clap::Parser;
  type Error: std::fmt::Display;

  /// Short name used for config-file discovery and log prefixes.
  fn app_name() -> &'static str;

  /// Construct a validated config from parsed CLI arguments.  This is
  /// where config-file loading, merging, and validation happen.
  fn from_cli(cli: Self::CliArgs) -> Result<Self, Self::Error>;

  /// Resolved log level for this invocation.
  fn log_level(&self) -> LogLevel;

  /// Resolved log format for this invocation.
  fn log_format(&self) -> LogFormat;
}
