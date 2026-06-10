//! Integration tests that drive the built binary.
//!
//! The executable path comes from cargo's `CARGO_BIN_EXE_*` env var rather than
//! walking `current_exe()`, which keeps the helper free of the brittle path
//! arithmetic (and the lint exemption it would need).

// Test code, where panicking is the desired failure signal.  clippy's in-test
// heuristic does not exempt free helper functions in an integration-test
// binary, so opt the whole file into the test allowances.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
  env!("CARGO_BIN_EXE_rust-template-compliance-cli")
}

fn manifest_path() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../compliance-checks.toml")
}

fn write(path: &Path, contents: &str) {
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent).unwrap();
  }
  std::fs::write(path, contents).unwrap();
}

fn registry_json(name: &str, dir: &Path, archived: bool) -> String {
  format!(
    r#"{{
  "templateSpawns": {{
    "{name}": {{
      "dir": "{}",
      "archived": {archived},
      "args": {{ "crates": "cli" }}
    }}
  }}
}}"#,
    dir.display()
  )
}

#[test]
fn help_succeeds() {
  let output = Command::new(bin()).arg("--help").output().unwrap();
  assert!(output.status.success());
  assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
}

#[test]
fn failing_spawn_exits_nonzero_and_names_check_ids() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let spawn = root.join("bare");
  // An empty spawn dir: every required file is missing.
  std::fs::create_dir_all(&spawn).unwrap();
  write(&root.join("config.json"), &registry_json("bare", &spawn, false));

  let output = Command::new(bin())
    .args(["--registry", root.join("config.json").to_str().unwrap()])
    .args(["--manifest", manifest_path().to_str().unwrap()])
    .args(["--template-dir", root.to_str().unwrap()])
    .args(["--format", "json"])
    .output()
    .unwrap();

  assert!(
    !output.status.success(),
    "a spawn missing required files should fail the run"
  );
  let stdout = String::from_utf8_lossy(&output.stdout);
  // The failing check's id and status are present in the JSON report.
  assert!(stdout.contains("required-file-changelog"), "stdout: {stdout}");
  assert!(stdout.contains("\"status\": \"fail\""), "stdout: {stdout}");
}

#[test]
fn run_with_no_applicable_checks_exits_zero() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  // The only spawn is archived, so no checks run and nothing can fail.
  write(
    &root.join("config.json"),
    &registry_json("old", &root.join("old"), true),
  );

  let output = Command::new(bin())
    .args(["--registry", root.join("config.json").to_str().unwrap()])
    .args(["--manifest", manifest_path().to_str().unwrap()])
    .args(["--template-dir", root.to_str().unwrap()])
    .output()
    .unwrap();

  assert!(
    output.status.success(),
    "stderr: {}",
    String::from_utf8_lossy(&output.stderr)
  );
}
