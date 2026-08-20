use crate::error::AppError;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

/// The changed files, and whether any of them is Rust source (which arms the
/// clippy gate).
pub struct Qualifying {
  pub files: Vec<String>,
  pub has_rust: bool,
}

/// Whether the current directory sits inside a git work tree.
pub fn inside_git_work_tree() -> bool {
  Command::new("git")
    .args(["rev-parse", "--is-inside-work-tree"])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|status| status.success())
}

/// Every path the working tree has changed relative to HEAD, deduplicated.
/// With a fixture in place the list is read from it instead, so tests never
/// touch a real tree.
pub fn changed_files(fixture: Option<&Path>) -> Result<Vec<String>, AppError> {
  fixture.map_or_else(git_changed_files, |path| {
    fs::read_to_string(path)
      .map(|listing| non_empty_lines(&listing).collect())
      .map_err(|source| AppError::WorkingTreeFixtureRead {
        path: path.to_path_buf(),
        source,
      })
  })
}

fn git_changed_files() -> Result<Vec<String>, AppError> {
  const LISTINGS: [&[&str]; 3] = [
    &["diff", "--name-only"],
    &["diff", "--cached", "--name-only"],
    &["ls-files", "--others", "--exclude-standard"],
  ];
  LISTINGS
    .iter()
    .map(|args| git_listing(args))
    .collect::<Result<Vec<_>, _>>()
    .map(|listings| {
      listings
        .into_iter()
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
    })
}

/// One `git` listing as its non-empty output lines.  `core.quotepath=false`
/// keeps non-ASCII paths literal rather than octal-escaped, so the path git
/// prints is the path `git hash-object` will accept back.  (`git -c` sets a
/// config value for this one invocation; git has no long-form spelling of the
/// flag.)
fn git_listing(args: &[&str]) -> Result<Vec<String>, AppError> {
  let command = format!("git {}", args.join(" "));
  let output = Command::new("git")
    .args(["-c", "core.quotepath=false"])
    .args(args)
    .stdin(Stdio::null())
    .output()
    .map_err(|source| AppError::WorkingTreeList {
      command: command.clone(),
      source,
    })?;
  if output.status.success() {
    Ok(non_empty_lines(&String::from_utf8_lossy(&output.stdout)).collect())
  } else {
    Err(AppError::WorkingTreeListFailed {
      command,
      stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
  }
}

fn non_empty_lines(text: &str) -> impl Iterator<Item = String> + '_ {
  text
    .lines()
    .filter(|line| !line.is_empty())
    .map(str::to_string)
}

/// Every changed file qualifies for review — prose as much as code, since
/// documentation carries conventions of its own.  The extension comparison is
/// case-insensitive so `MAIN.RS` still arms the clippy gate.
pub fn qualifying(changed: Vec<String>) -> Qualifying {
  Qualifying {
    has_rust: changed
      .iter()
      .any(|path| path.to_lowercase().ends_with(".rs")),
    files: changed,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_changed_file_qualifies() {
    let qualifying = qualifying(
      ["README.org", "LICENSE", "flake.nix"]
        .into_iter()
        .map(str::to_string)
        .collect(),
    );
    assert_eq!(qualifying.files.len(), 3);
    assert!(!qualifying.has_rust);
  }

  #[test]
  fn rust_sources_arm_the_clippy_gate() {
    let qualifying =
      qualifying(vec!["src/MAIN.RS".to_string(), "README.org".to_string()]);
    assert!(qualifying.has_rust);
  }
}
