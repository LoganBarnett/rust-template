//! Tests for LogLevel and LogFormat parsing.

use rust_template_foundation::logging::{LogFormat, LogLevel};

#[test]
fn parse_log_levels() {
  assert_eq!("trace".parse::<LogLevel>().unwrap(), LogLevel::Trace);
  assert_eq!("DEBUG".parse::<LogLevel>().unwrap(), LogLevel::Debug);
  assert_eq!("Info".parse::<LogLevel>().unwrap(), LogLevel::Info);
  assert_eq!("warn".parse::<LogLevel>().unwrap(), LogLevel::Warn);
  assert_eq!("warning".parse::<LogLevel>().unwrap(), LogLevel::Warn);
  assert_eq!("error".parse::<LogLevel>().unwrap(), LogLevel::Error);
}

#[test]
fn parse_invalid_log_level() {
  let err = "invalid".parse::<LogLevel>().unwrap_err();
  assert!(err.to_string().contains("Invalid log level"));
}

#[test]
fn parse_log_formats() {
  assert_eq!("text".parse::<LogFormat>().unwrap(), LogFormat::Text);
  assert_eq!("pretty".parse::<LogFormat>().unwrap(), LogFormat::Text);
  assert_eq!("json".parse::<LogFormat>().unwrap(), LogFormat::Json);
  assert_eq!("JSON".parse::<LogFormat>().unwrap(), LogFormat::Json);
}

#[test]
fn parse_invalid_log_format() {
  let err = "xml".parse::<LogFormat>().unwrap_err();
  assert!(err.to_string().contains("Invalid log format"));
}

#[test]
fn log_level_display_roundtrip() {
  for level in [
    LogLevel::Trace,
    LogLevel::Debug,
    LogLevel::Info,
    LogLevel::Warn,
    LogLevel::Error,
  ] {
    assert_eq!(level.to_string().parse::<LogLevel>().unwrap(), level);
  }
}

#[test]
fn log_format_display_roundtrip() {
  for format in [LogFormat::Text, LogFormat::Json] {
    assert_eq!(format.to_string().parse::<LogFormat>().unwrap(), format);
  }
}

#[test]
fn log_level_to_tracing_level() {
  let level: tracing::Level = LogLevel::Info.into();
  assert_eq!(level, tracing::Level::INFO);
}
