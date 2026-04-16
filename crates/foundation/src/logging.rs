//! Log level and format types shared across CLI and server crates.
//!
//! The initialisation functions (`init_cli_logging`, `init_server_logging`)
//! are feature-gated so both can coexist when a crate enables `cli` and
//! `server` simultaneously.

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// Severity threshold for log output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
  Trace,
  Debug,
  Info,
  Warn,
  Error,
}

impl FromStr for LogLevel {
  type Err = LogLevelParseError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s.to_lowercase().as_str() {
      "trace" => Ok(LogLevel::Trace),
      "debug" => Ok(LogLevel::Debug),
      "info" => Ok(LogLevel::Info),
      "warn" | "warning" => Ok(LogLevel::Warn),
      "error" => Ok(LogLevel::Error),
      _ => Err(LogLevelParseError::InvalidLevel(s.to_string())),
    }
  }
}

impl From<LogLevel> for tracing::Level {
  fn from(level: LogLevel) -> Self {
    match level {
      LogLevel::Trace => tracing::Level::TRACE,
      LogLevel::Debug => tracing::Level::DEBUG,
      LogLevel::Info => tracing::Level::INFO,
      LogLevel::Warn => tracing::Level::WARN,
      LogLevel::Error => tracing::Level::ERROR,
    }
  }
}

impl std::fmt::Display for LogLevel {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      LogLevel::Trace => write!(f, "trace"),
      LogLevel::Debug => write!(f, "debug"),
      LogLevel::Info => write!(f, "info"),
      LogLevel::Warn => write!(f, "warn"),
      LogLevel::Error => write!(f, "error"),
    }
  }
}

/// Structured log encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
  Text,
  Json,
}

impl FromStr for LogFormat {
  type Err = LogFormatParseError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s.to_lowercase().as_str() {
      "text" | "pretty" => Ok(LogFormat::Text),
      "json" => Ok(LogFormat::Json),
      _ => Err(LogFormatParseError::InvalidFormat(s.to_string())),
    }
  }
}

impl std::fmt::Display for LogFormat {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      LogFormat::Text => write!(f, "text"),
      LogFormat::Json => write!(f, "json"),
    }
  }
}

#[derive(Debug, Error)]
pub enum LogLevelParseError {
  #[error(
    "Invalid log level: {0}. Valid values are: trace, debug, info, warn, error"
  )]
  InvalidLevel(String),
}

#[derive(Debug, Error)]
pub enum LogFormatParseError {
  #[error("Invalid log format: {0}. Valid values are: text, json")]
  InvalidFormat(String),
}

/// Initialise CLI logging — writes to stderr so program output on stdout
/// remains clean for piping.
#[cfg(feature = "cli")]
pub fn init_cli_logging(level: LogLevel, format: LogFormat) {
  use tracing_subscriber::{
    fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
  };

  let env_filter = EnvFilter::try_from_default_env()
    .unwrap_or_else(|_| EnvFilter::new(level.to_string()));

  match format {
    LogFormat::Text => {
      tracing_subscriber::registry()
        .with(
          fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_line_number(true)
            .with_filter(env_filter),
        )
        .init();
    }
    LogFormat::Json => {
      tracing_subscriber::registry()
        .with(
          fmt::layer()
            .json()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_line_number(true)
            .with_filter(env_filter),
        )
        .init();
    }
  }
}

/// Initialise server logging — tries journald on Unix first, falls back
/// to stderr with the requested format.
#[cfg(feature = "server")]
pub fn init_server_logging(level: LogLevel, format: LogFormat) {
  use tracing_subscriber::{
    fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
  };

  let env_filter = EnvFilter::try_from_default_env()
    .unwrap_or_else(|_| EnvFilter::new(level.to_string()));

  #[cfg(unix)]
  if let Ok(journald) = tracing_journald::layer() {
    tracing_subscriber::registry()
      .with(tracing_subscriber::Layer::with_filter(journald, env_filter))
      .init();
    return;
  }

  match format {
    LogFormat::Text => {
      tracing_subscriber::registry()
        .with(
          fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_line_number(true)
            .with_filter(env_filter),
        )
        .init();
    }
    LogFormat::Json => {
      tracing_subscriber::registry()
        .with(
          fmt::layer()
            .json()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_line_number(true)
            .with_filter(env_filter),
        )
        .init();
    }
  }
}
