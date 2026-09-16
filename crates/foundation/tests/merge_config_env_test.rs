//! Tests for the env-var binding of `MergeConfig` merged fields.
//!
//! Every merged field reads `<app>_<flag>` unless it opts out with `no_env`
//! or names its own variable with `env = "..."`; these cases pin each of the
//! three down through the generated `CliRaw`.

use clap::{CommandFactory, Parser};
use rust_template_foundation::logging::{LogFormat, LogLevel};
use rust_template_foundation::MergeConfig;
use std::sync::Mutex;

/// The environment is process-global state and the harness runs tests on
/// parallel threads, so every test that mutates it serializes on this lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, MergeConfig)]
#[merge_config(app_name = "env-test")]
pub struct Config {
  #[merge_config(common)]
  pub log_level: LogLevel,
  #[merge_config(common)]
  pub log_format: LogFormat,
  /// A field with no env attribute reads the derived variable.
  #[merge_config(default = "8080")]
  pub port: u16,
  /// A secret that must come from the command line or the file only.
  #[merge_config(no_env, default = "String::new()")]
  pub token: String,
  /// A name imposed by an external contract, so it is spelled out.
  #[merge_config(env = "ENV_TEST_LEGACY", default = "String::new()")]
  pub legacy: String,
  /// The derived name follows the flag, not the field identifier.
  #[merge_config(name = "listen", default = "String::new()")]
  pub listen_address: String,
}

/// The env var clap reads for the argument with this id, if any.
fn env_of(id: &str) -> Option<String> {
  CliRaw::command()
    .get_arguments()
    .find(|arg| arg.get_id().as_str() == id)
    .and_then(|arg| arg.get_env())
    .map(|name| name.to_string_lossy().into_owned())
}

#[test]
fn every_merged_field_reads_the_derived_variable_unless_told_otherwise() {
  assert_eq!(env_of("port").as_deref(), Some("env_test_port"));
  assert_eq!(env_of("listen").as_deref(), Some("env_test_listen"));
  assert_eq!(env_of("legacy").as_deref(), Some("ENV_TEST_LEGACY"));
  assert_eq!(env_of("token"), None);
}

#[test]
fn the_common_fields_read_their_prefixed_variables() {
  assert_eq!(env_of("log_level").as_deref(), Some("env_test_log_level"));
  assert_eq!(env_of("log_format").as_deref(), Some("env_test_log_format"));
  assert_eq!(env_of("config").as_deref(), Some("env_test_config"));
}

#[test]
fn the_environment_reaches_the_resolved_config() {
  let _guard = ENV_LOCK
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  std::env::set_var("env_test_port", "9090");
  std::env::set_var("env_test_token", "leak");
  std::env::set_var("ENV_TEST_LEGACY", "old");
  std::env::set_var("env_test_listen", "0.0.0.0:1");

  let from_env =
    Config::from_cli_and_file(CliRaw::try_parse_from(["env-test"]).unwrap())
      .unwrap();
  let from_flag = Config::from_cli_and_file(
    CliRaw::try_parse_from(["env-test", "--port", "1"]).unwrap(),
  )
  .unwrap();

  std::env::remove_var("env_test_port");
  std::env::remove_var("env_test_token");
  std::env::remove_var("ENV_TEST_LEGACY");
  std::env::remove_var("env_test_listen");

  assert_eq!(from_env.port, 9090);
  assert_eq!(from_env.legacy, "old");
  assert_eq!(from_env.listen_address, "0.0.0.0:1");
  assert_eq!(from_env.token, "", "a no_env field ignores its variable");
  assert_eq!(from_flag.port, 1, "a flag beats the variable");
}
