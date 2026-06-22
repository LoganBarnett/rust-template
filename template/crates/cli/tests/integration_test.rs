// Integration tests under tests/ may use the panicking variants (unwrap,
// expect, panic) freely — see llms.org's "No unwrap or expect" test exemption.
// clippy's is_in_test heuristic does not recognize tests/ integration tests as
// test code, so the workspace-level denials reach them and must be allowed at
// the file level.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::{path::PathBuf, process::Command};

fn get_binary_path() -> PathBuf {
  let mut path =
    std::env::current_exe().expect("Failed to get current executable path");

  // Navigate from the test executable to the binary
  path.pop(); // remove test executable name
  path.pop(); // remove deps dir
  path.push("rust-template-cli");

  // If the binary doesn't exist in release, try debug
  if !path.exists() {
    path.pop();
    path.pop();
    path.push("debug");
    path.push("rust-template-cli");
  }

  path
}

#[test]
fn test_help_flag() {
  let output = Command::new(get_binary_path()).arg("--help").output();

  match output {
    Ok(output) => {
      assert!(
        output.status.success(),
        "Expected success exit code, got: {:?}",
        output.status.code()
      );
      let stdout = String::from_utf8_lossy(&output.stdout);
      assert!(
        stdout.contains("Usage:"),
        "Expected help text to contain 'Usage:', got: {}",
        stdout
      );
    }
    Err(e) => {
      if e.kind() == std::io::ErrorKind::NotFound {
        eprintln!(
                    "Binary not found. Please build the project first with: cargo build -p rust-template-cli"
                );
      }
      panic!("Failed to execute binary: {}", e);
    }
  }
}

#[test]
fn test_version_flag() {
  let output = Command::new(get_binary_path()).arg("--version").output();

  match output {
    Ok(output) => {
      assert!(
        output.status.success(),
        "Expected success exit code, got: {:?}",
        output.status.code()
      );
      let stdout = String::from_utf8_lossy(&output.stdout);
      // clap prints the app name (the `merge_config(app_name = ...)` value,
      // which also drives env-var prefixes), not the binary name, so the
      // version line reads "<app> <version>".
      assert!(
        stdout.contains("rust-template"),
        "Expected version text to contain 'rust-template', got: {}",
        stdout
      );
    }
    Err(e) => {
      if e.kind() == std::io::ErrorKind::NotFound {
        eprintln!(
                    "Binary not found. Please build the project first with: cargo build -p rust-template-cli"
                );
      }
      panic!("Failed to execute binary: {}", e);
    }
  }
}

#[test]
fn test_basic_execution() {
  let output = Command::new(get_binary_path()).output();

  match output {
    Ok(output) => {
      assert!(
        output.status.success(),
        "Expected success exit code, got: {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
      );
    }
    Err(e) => {
      if e.kind() == std::io::ErrorKind::NotFound {
        eprintln!(
                    "Binary not found. Please build the project first with: cargo build -p rust-template-cli"
                );
      }
      panic!("Failed to execute binary: {}", e);
    }
  }
}

#[test]
fn test_with_name_argument() {
  let output = Command::new(get_binary_path())
    .arg("--name")
    .arg("Rust")
    .output();

  match output {
    Ok(output) => {
      assert!(
        output.status.success(),
        "Expected success exit code, got: {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
      );
    }
    Err(e) => {
      if e.kind() == std::io::ErrorKind::NotFound {
        eprintln!(
                    "Binary not found. Please build the project first with: cargo build -p rust-template-cli"
                );
      }
      panic!("Failed to execute binary: {}", e);
    }
  }
}
