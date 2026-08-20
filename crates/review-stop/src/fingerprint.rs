use crate::error::AppError;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// A fingerprint of the changed files' current content — one line per path
/// with its blob hash, hashed together — so staged-versus-unstaged makes no
/// difference.  A file that no longer
/// exists (a deletion) hashes as "absent".  With the fixture override in place
/// this runs over the fixture's paths just the same, so a different fixture is
/// a different fingerprint.
///
/// The manifest keeps the input order so the same tree always yields the same
/// fingerprint.
pub fn of(files: &[String]) -> Result<String, AppError> {
  let present: Vec<&str> = files
    .iter()
    .map(String::as_str)
    .filter(|path| Path::new(path).exists())
    .collect();
  let hashes: HashMap<&str, String> = present
    .iter()
    .copied()
    .zip(blob_hashes(&present)?)
    .collect();
  git_hash_object(
    &["--stdin"],
    &files
      .iter()
      .map(|path| {
        format!(
          "{path} {}\n",
          hashes.get(path.as_str()).map_or("absent", String::as_str)
        )
      })
      .collect::<String>(),
  )
  .map(|hashed| hashed.lines().next().unwrap_or_default().to_string())
}

/// The blob hash of each path, in order.  Paths go on the command line rather
/// than through `--stdin-paths`, which git resolves against the repository
/// root instead of the current directory; they are chunked so a large working
/// tree never runs into the argument-length limit.
fn blob_hashes(paths: &[&str]) -> Result<Vec<String>, AppError> {
  const CHUNK: usize = 256;
  let hashes: Vec<String> = paths
    .chunks(CHUNK)
    .map(|chunk| {
      git_hash_object(
        &["--"]
          .into_iter()
          .chain(chunk.iter().copied())
          .collect::<Vec<_>>(),
        "",
      )
      .map(|output| output.lines().map(str::to_string).collect::<Vec<_>>())
    })
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    .flatten()
    .collect();
  if hashes.len() == paths.len() {
    Ok(hashes)
  } else {
    Err(AppError::FingerprintHashCount {
      hashes: hashes.len(),
      paths: paths.len(),
    })
  }
}

/// Run `git hash-object <args>` with `input` on stdin and return its stdout.
fn git_hash_object(args: &[&str], input: &str) -> Result<String, AppError> {
  let mut child = Command::new("git")
    .arg("hash-object")
    .args(args)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|source| AppError::FingerprintHash { source })?;
  // The pipe is dropped at the end of this block so git sees end-of-input;
  // `take` leaves the child without a handle to it, and a child spawned with a
  // piped stdin always has one to take.
  child
    .stdin
    .take()
    .map_or(Ok(()), |mut stdin| stdin.write_all(input.as_bytes()))
    .map_err(|source| AppError::FingerprintHash { source })?;
  let output = child
    .wait_with_output()
    .map_err(|source| AppError::FingerprintHash { source })?;
  if output.status.success() {
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
  } else {
    Err(AppError::FingerprintHashFailed {
      stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
  }
}
