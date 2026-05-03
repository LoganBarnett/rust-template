//! Tests for config file discovery and TOML loading.

use rust_template_foundation::config::{
  find_config_file, load_toml, ConfigFileError,
};
use serde::Deserialize;
use std::path::PathBuf;
use tempfile::TempDir;

#[derive(Debug, Deserialize, PartialEq)]
struct SimpleConfig {
  name: Option<String>,
  count: Option<u32>,
}

// ── load_toml ───────────────────────────────────────────────────────────────

#[test]
fn load_toml_valid_file() {
  let dir = TempDir::new().unwrap();
  let path = dir.path().join("config.toml");
  std::fs::write(&path, "name = \"hello\"\ncount = 42\n").unwrap();

  let config: SimpleConfig = load_toml(&path).unwrap();
  assert_eq!(config.name, Some("hello".to_string()));
  assert_eq!(config.count, Some(42));
}

#[test]
fn load_toml_missing_file() {
  let result: Result<SimpleConfig, _> =
    load_toml(&PathBuf::from("/nonexistent/config.toml"));
  assert!(matches!(result, Err(ConfigFileError::FileRead { .. })));
}

#[test]
fn load_toml_invalid_toml() {
  let dir = TempDir::new().unwrap();
  let path = dir.path().join("bad.toml");
  std::fs::write(&path, "not valid toml [[[").unwrap();

  let result: Result<SimpleConfig, _> = load_toml(&path);
  assert!(matches!(result, Err(ConfigFileError::Parse { .. })));
}

// ── find_config_file ────────────────────────────────────────────────────────

#[test]
fn find_config_file_explicit_path() {
  let dir = TempDir::new().unwrap();
  let explicit = dir.path().join("custom.toml");
  std::fs::write(&explicit, "").unwrap();

  let result = find_config_file("test-app", Some(explicit.as_path()));
  assert_eq!(result, Some(explicit));
}

#[test]
fn find_config_file_explicit_path_even_if_missing() {
  // Explicit path is returned unconditionally — caller decides whether to
  // error on missing.
  let missing = PathBuf::from("/tmp/does-not-exist-config.toml");
  let result = find_config_file("test-app", Some(missing.as_path()));
  assert_eq!(result, Some(missing));
}

#[test]
fn find_config_file_xdg_discovery() {
  let dir = TempDir::new().unwrap();
  let app_dir = dir.path().join("test-app");
  std::fs::create_dir_all(&app_dir).unwrap();
  std::fs::write(app_dir.join("config.toml"), "").unwrap();

  // Override XDG_CONFIG_HOME for the duration of this test.
  std::env::set_var("XDG_CONFIG_HOME", dir.path());

  let result = find_config_file("test-app", None);
  assert_eq!(result, Some(app_dir.join("config.toml")));

  std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn find_config_file_none_when_nothing_exists() {
  // Point XDG somewhere empty so the lookup can't pick up real config.
  let dir = TempDir::new().unwrap();
  std::env::set_var("XDG_CONFIG_HOME", dir.path());

  let result = find_config_file("nonexistent-app-xyz-test", None);
  assert!(result.is_none());

  std::env::remove_var("XDG_CONFIG_HOME");
}
