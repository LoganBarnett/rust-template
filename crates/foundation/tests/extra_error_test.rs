//! Tests for the `merge_config(extra_error = "...")` struct attribute.
//!
//! When `extra_error` is set, the generated `ConfigError` gains an
//! `Extra(T)` variant with `From<T>` and transparent `Display`, so a
//! user-written `resolve_<field>` returning `Result<_, T>` propagates
//! through the `?` inside the generated `from_cli_and_file`.

use clap::Parser;
use rust_template_foundation::logging::{LogFormat, LogLevel};
use rust_template_foundation::MergeConfig;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
enum MyError {
  #[error("port {0} is reserved")]
  ReservedPort(u16),
  #[error("missing widget catalogue")]
  MissingCatalogue,
}

#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "extra-error-test", extra_error = "crate::MyError")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  #[merge_config(default = "8080")]
  pub port: u16,
  #[merge_config(skip)]
  pub catalogue: String,
}

impl Config {
  fn resolve_catalogue(
    _cli: &CliRaw,
    _file: &ConfigFileRaw,
  ) -> Result<String, MyError> {
    Err(MyError::MissingCatalogue)
  }
}

#[test]
fn user_error_propagates_as_extra_variant() {
  let cli = CliRaw::try_parse_from(["extra-error-test"]).unwrap();
  let err = Config::from_cli_and_file(cli).unwrap_err();
  match err {
    ConfigError::Extra(MyError::MissingCatalogue) => {}
    other => panic!("expected Extra(MissingCatalogue), got {:?}", other),
  }
}

#[test]
fn extra_variant_display_is_transparent() {
  let inner = MyError::ReservedPort(22);
  let inner_display = inner.to_string();
  let wrapped = ConfigError::Extra(inner);
  assert_eq!(wrapped.to_string(), inner_display);
}

#[test]
fn from_user_error_into_config_error() {
  // `?` in user-written code uses this conversion.
  let err: ConfigError = MyError::ReservedPort(80).into();
  assert!(matches!(err, ConfigError::Extra(MyError::ReservedPort(80))));
}
