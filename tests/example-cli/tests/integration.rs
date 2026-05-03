use std::process::Command;

fn binary_path() -> std::path::PathBuf {
  std::path::PathBuf::from(env!("CARGO_BIN_EXE_example-cli"))
}

#[test]
fn help_flag_works() {
  let output = Command::new(binary_path())
    .arg("--help")
    .output()
    .expect("failed to execute binary");

  assert!(output.status.success(), "exit code should be 0");
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert!(
    stdout.contains("example-cli") || stdout.contains("Usage"),
    "help should contain program name or usage"
  );
}

#[test]
fn basic_execution() {
  let output = Command::new(binary_path())
    .output()
    .expect("failed to execute binary");

  assert!(output.status.success(), "exit code should be 0");
}

#[test]
fn name_argument() {
  let output = Command::new(binary_path())
    .args(["--name", "Rust"])
    .output()
    .expect("failed to execute binary");

  assert!(output.status.success(), "exit code should be 0");
}
