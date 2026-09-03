use crate::error::{AppError, GitFailure};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

const UNTRACKED_LISTING: &[&str] =
  &["ls-files", "--others", "--exclude-standard"];

fn git_changed_files() -> Result<Vec<String>, AppError> {
  // `git status --porcelain=v1` is git's committed-stable, script-facing
  // format, unlike `git diff --name-only` — a porcelain command whose output
  // format carries no such guarantee.  `-z` NUL-separates records so no path
  // is ever quoted, and `--untracked-files=all` lists untracked files one by
  // one rather than collapsing them to their directory.
  let output = git_query(
    &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    AppError::WorkingTreeList,
  )?;
  if output.status.success() {
    Ok(porcelain_paths(&String::from_utf8_lossy(&output.stdout)))
  } else {
    Err(AppError::WorkingTreeList(git_exit(&output)))
  }
}

/// The changed paths from `git status --porcelain=v1 -z`, deduplicated.
fn porcelain_paths(output: &str) -> Vec<String> {
  output
    .split('\0')
    .filter(|record| !record.is_empty())
    // Each record is a two-character status, a space, then the path, so the
    // path begins at byte three — except a rename or copy (status `R`/`C`),
    // whose original path follows as a bare record with no status prefix.
    // `scan` carries a one-record flag so that trailing origin record is
    // dropped rather than mis-parsed; its path is not needed, since the current
    // path already names the change.
    .scan(false, |skip_origin, record| {
      let path = (!*skip_origin).then(|| record.get(3..)).flatten();
      *skip_origin = path.is_some() && is_rename_or_copy(record);
      Some(path)
    })
    .flatten()
    .map(str::to_string)
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect()
}

/// Whether a porcelain status record is a rename or copy, which carries its
/// original path as the following record.
fn is_rename_or_copy(record: &str) -> bool {
  let status = record.as_bytes();
  matches!(status.first(), Some(b'R' | b'C'))
    || matches!(status.get(1), Some(b'R' | b'C'))
}

/// The files present in the working tree that git does not track and does
/// not ignore.
pub fn untracked_files() -> Result<Vec<String>, AppError> {
  git_listing(UNTRACKED_LISTING, AppError::PacketUntrackedList)
}

/// One `git` listing as its non-empty output lines; `on_fail` names the
/// operation the listing serves.
fn git_listing(
  args: &[&str],
  on_fail: impl Fn(GitFailure) -> AppError,
) -> Result<Vec<String>, AppError> {
  let output = git_query(args, &on_fail)?;
  if output.status.success() {
    Ok(non_empty_lines(&String::from_utf8_lossy(&output.stdout)).collect())
  } else {
    Err(on_fail(git_exit(&output)))
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

/// The absolute path of the repository's top-level directory, which names the
/// verdict cache directory.
pub fn toplevel() -> Result<String, AppError> {
  git_output(&["rev-parse", "--show-toplevel"], AppError::CacheKeyToplevel)
}

/// The commit `HEAD` names, or `None` on an unborn branch (a repository with
/// no commits yet).
pub fn head_commit() -> Result<Option<String>, AppError> {
  let output = git_query(
    &["rev-parse", "--verify", "--quiet", "HEAD"],
    AppError::HeadResolve,
  )?;
  if output.status.success() {
    Ok(Some(String::from_utf8_lossy(&output.stdout).trim().to_string()))
    // For a repository with no commits yet, `rev-parse --verify --quiet`
    // reports as a silent exit of one rather than an error.
  } else if output.status.code() == Some(1) && output.stderr.is_empty() {
    Ok(None)
  } else {
    Err(AppError::HeadResolve(git_exit(&output)))
  }
}

/// The hash of the empty tree, the diff base that shows every file as added
/// when there is no commit to diff against.  Asked of git rather than
/// hard-coded so a SHA-256 repository gets its own value.
pub fn empty_tree() -> Result<String, AppError> {
  git_output(
    // `hash-object -t` names the object type; git has no long-form spelling of
    // the flag.
    &["hash-object", "-t", "tree", "/dev/null"],
    AppError::EmptyTreeHash,
  )
}

/// The working tree's diff against `base`, staged and unstaged together, with
/// external diff drivers bypassed so the output is always unified text.
pub fn diff_against(base: &str) -> Result<String, AppError> {
  let output = git_query(
    &["diff", "--no-color", "--no-ext-diff", base],
    AppError::PacketDiff,
  )?;
  if output.status.success() {
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
  } else {
    Err(AppError::PacketDiff(git_exit(&output)))
  }
}

/// The content of `path` as committed at `HEAD`, or `None` when that commit
/// has no such file.
pub fn committed_file(path: &str) -> Result<Option<String>, AppError> {
  let on_fail = |source| AppError::ConventionShow {
    path: PathBuf::from(path),
    source,
  };
  let output = git_query(&["show", &format!("HEAD:{path}")], on_fail)?;
  let stderr = String::from_utf8_lossy(&output.stderr);
  if output.status.success() {
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
    // Git reports the missing case as a failure with a recognisable message;
    // any other failure is a real error.
  } else if stderr.contains("does not exist in")
    || stderr.contains("exists on disk, but not in")
  {
    Ok(None)
  } else {
    Err(on_fail(git_exit(&output)))
  }
}

/// Run `git <args>` and return its output for the caller to classify.
/// `on_fail` names the operation; here it wraps only a launch failure, since a
/// non-zero exit is left to the caller — several callers read a specific
/// failure as a valid outcome (an unborn branch, a path absent at `HEAD`).
fn git_query(
  args: &[&str],
  on_fail: impl Fn(GitFailure) -> AppError,
) -> Result<Output, AppError> {
  git_command(args)
    .output()
    .map_err(|source| on_fail(GitFailure::Io(source)))
}

/// The trimmed stdout of `git <args>`, which must succeed; `on_fail` names the
/// operation on either failure mode.
fn git_output(
  args: &[&str],
  on_fail: impl Fn(GitFailure) -> AppError,
) -> Result<String, AppError> {
  let output = git_query(args, &on_fail)?;
  if output.status.success() {
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
  } else {
    Err(on_fail(git_exit(&output)))
  }
}

/// The exit-failure cause for a `git` command that ran but returned non-zero.
fn git_exit(output: &Output) -> GitFailure {
  GitFailure::Exit(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

/// A `git` invocation with the config every call in this module shares.
/// `core.quotepath=false` keeps non-ASCII paths literal rather than
/// octal-escaped, so a path git prints is the path `git hash-object` accepts
/// back.
fn git_command(args: &[&str]) -> Command {
  let mut command = Command::new("git");
  command
    // `git -c` sets a config value for this one invocation; git has no
    // long-form spelling of the flag.
    .args(["-c", "core.quotepath=false"])
    .args(args)
    .stdin(Stdio::null());
  command
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

  #[test]
  fn porcelain_parse_reads_paths_and_consumes_rename_origins() {
    // A modified file, an untracked file, and a staged rename whose origin
    // path follows as its own record.
    let output = " M src/main.rs\0?? notes.txt\0R  new.rs\0old.rs\0";
    assert_eq!(
      porcelain_paths(output),
      vec![
        "new.rs".to_string(),
        "notes.txt".to_string(),
        "src/main.rs".to_string(),
      ]
    );
  }
}
