//! The impure half of the bump: every subprocess and file write the engine
//! performs.
//!
//! Funnelling the process environment through one module keeps the pure
//! modules unit-testable without one.

use crate::audit::{self, Advisories, AuditProbeError};
use crate::compose::{self, Entry};
use crate::error::DependencyBumpError;
use crate::holds::{self, Hold};
use crate::lockfile::{self, Bump, Snapshot};
use crate::runners::{self, RunnerBump, RunnerImageProbeError, WorkflowFile};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// What a run was asked to do.
#[derive(Debug, Clone)]
pub struct RunOptions {
  /// Workspace to bump (its root holds Cargo.toml and Cargo.lock).
  pub workspace_dir: PathBuf,
  /// Changelog file, relative to the workspace; skipped when absent.
  pub changelog_file: String,
  /// TSV report destination; `None` writes no report.
  pub report_file: Option<PathBuf>,
  /// Where the runner-images "Available Images" table is fetched from.
  pub runner_images_readme_url: String,
  /// Preview what would move without touching anything.
  pub dry_run: bool,
}

/// One landed bump with its composed changelog entry.
#[derive(Debug, Clone)]
pub struct AppliedBump {
  pub name: String,
  pub from: String,
  pub to: String,
  pub entry: Entry,
}

/// What a run did.
#[derive(Debug, Clone)]
pub struct BumpOutcome {
  pub bumps: Vec<AppliedBump>,
  pub held: Vec<Hold>,
  pub changelog_updated: bool,
  /// The runner-label advances a dry run would apply; empty otherwise.
  pub planned_runner_bumps: Vec<AppliedBump>,
}

/// Runs the bump and reports what moved.  The report file is the only
/// machine contract — it is written empty when nothing moved, and not at
/// all on `dry_run`.
pub fn run(options: &RunOptions) -> Result<BumpOutcome, DependencyBumpError> {
  let manifest_path = options.workspace_dir.join("Cargo.toml");
  let lockfile_path = options.workspace_dir.join("Cargo.lock");
  let held = holds::holds(&manifest_path)?;
  held.iter().for_each(|hold| {
    tracing::info!(
      package = %hold.package,
      reason = %hold.reason,
      "hold declared; this package will not advance"
    );
  });
  let advisories = advisories_or_empty(&options.workspace_dir);
  let workflows = workflow_files(&options.workspace_dir)?;
  let runner_bumps =
    runner_bumps_or_none(&workflows, &options.runner_images_readme_url);

  if options.dry_run {
    cargo_update(&options.workspace_dir, true)?;
    return Ok(BumpOutcome {
      bumps: Vec::new(),
      held,
      changelog_updated: false,
      planned_runner_bumps: runner_bumps
        .iter()
        .map(applied_runner_bump)
        .collect(),
    });
  }

  require_clean_lockfile(&options.workspace_dir)?;
  let before = lockfile::snapshot(&lockfile_path)?;
  cargo_update(&options.workspace_dir, false)?;
  repin_held(&options.workspace_dir, &held, &before, &lockfile_path)?;

  let applied =
    lockfile::bumps_between(&before, &lockfile::snapshot(&lockfile_path)?)
      .iter()
      .map(|bump| AppliedBump {
        name: bump.name.clone(),
        from: bump.from.clone(),
        to: bump.to.clone(),
        entry: compose::entry(bump, &advisories),
      })
      .chain(apply_runner_bumps(&workflows, &runner_bumps)?)
      .collect::<Vec<_>>();

  let changelog_updated = (!applied.is_empty())
    .then(|| {
      update_changelog(
        &options.workspace_dir,
        &options.changelog_file,
        &applied,
      )
    })
    .transpose()?
    .unwrap_or(false);

  options
    .report_file
    .as_deref()
    .map(|path| write_report(path, &applied))
    .transpose()?;

  Ok(BumpOutcome {
    bumps: applied,
    held,
    changelog_updated,
    planned_runner_bumps: Vec::new(),
  })
}

/// The advisory set, or empty when `cargo audit` cannot produce one — an
/// advisory-database outage downgrades classification to Maintenance, it
/// must never block the bump itself.
fn advisories_or_empty(workspace_dir: &Path) -> Advisories {
  audit_probe(workspace_dir).unwrap_or_else(|error| {
    tracing::warn!(
      %error,
      "cargo audit produced no readable report; every bump will be filed \
       under Maintenance"
    );
    Advisories::new()
  })
}

/// Runs `cargo audit --json` and parses its report.  `cargo audit` exits
/// non-zero when it FINDS advisories, so the exit status cannot separate
/// "found some" from "audit broke"; parseable JSON on stdout is the
/// success signal instead, which is why the status goes unexamined here.
fn audit_probe(workspace_dir: &Path) -> Result<Advisories, AuditProbeError> {
  Command::new("cargo")
    .args(["audit", "--json"])
    .current_dir(workspace_dir)
    .stderr(Stdio::null())
    .output()
    .map_err(AuditProbeError::from)
    .and_then(|output| String::from_utf8(output.stdout).map_err(Into::into))
    .and_then(|json| audit::parse_advisories(&json).map_err(Into::into))
}

/// Every workflow file's text, in path order.  Nothing pins a runner label
/// outside `.github/workflows`, so a workspace without that directory yields
/// nothing rather than an error.
fn workflow_files(
  workspace_dir: &Path,
) -> Result<Vec<WorkflowFile>, DependencyBumpError> {
  let dir = workspace_dir.join(".github").join("workflows");
  if !dir.is_dir() {
    return Ok(Vec::new());
  }
  std::fs::read_dir(&dir)
    .map_err(|source| DependencyBumpError::WorkflowDirReadError {
      path: dir.clone(),
      source,
    })?
    .map(|entry| {
      entry.map(|entry| entry.path()).map_err(|source| {
        DependencyBumpError::WorkflowDirReadError {
          path: dir.clone(),
          source,
        }
      })
    })
    .filter(|path| path.as_ref().map_or(true, |path| is_workflow(path)))
    .map(|path| path.and_then(read_workflow))
    .collect::<Result<Vec<WorkflowFile>, DependencyBumpError>>()
    .map(|mut files| {
      files.sort_by(|left, right| left.path.cmp(&right.path));
      files
    })
}

fn is_workflow(path: &Path) -> bool {
  path.is_file()
    && path
      .extension()
      .is_some_and(|extension| extension == "yml" || extension == "yaml")
}

fn read_workflow(path: PathBuf) -> Result<WorkflowFile, DependencyBumpError> {
  std::fs::read_to_string(&path)
    .map(|text| WorkflowFile {
      path: path.clone(),
      text,
    })
    .map_err(|source| DependencyBumpError::WorkflowFileReadError {
      path,
      source,
    })
}

/// The runner-label advances, or none.  A runner-images outage must not
/// block the cargo bumps, so an unreadable README degrades to a warning
/// and the labels stay.  Spawn workflows carry no versioned runner label
/// (their CI is delegated to rust-template's reusables), so the common case
/// skips the fetch altogether.
fn runner_bumps_or_none(
  files: &[WorkflowFile],
  readme_url: &str,
) -> Vec<RunnerBump> {
  let pinned = runners::pinned(files);
  if pinned.is_empty() {
    tracing::debug!(
      "no versioned ubuntu runner label in the workflows; skipping the \
       runner-images lookup"
    );
    return Vec::new();
  }
  fetch_readme(readme_url)
    .and_then(|readme| runners::newest_available(&readme))
    .map_or_else(
      |error| {
        tracing::warn!(
          %error,
          "could not determine the newest runner image; the runner labels \
           stay as they are"
        );
        Vec::new()
      },
      |available| runners::plan(&pinned, &available),
    )
}

/// Fetches the runner-images README through curl.  curl exits 0 on an HTTP
/// error and hands back the error page as the body, which would read as a
/// table with no ubuntu rows; `--fail` turns that into a fetch error.
fn fetch_readme(url: &str) -> Result<String, RunnerImageProbeError> {
  Command::new("curl")
    .args(["--silent", "--show-error", "--fail", "--location", "--"])
    .arg(url)
    .output()
    .map_err(RunnerImageProbeError::from)
    .and_then(
      |Output {
         status,
         stdout,
         stderr,
       }| {
        status.success().then_some(stdout).ok_or_else(|| {
          RunnerImageProbeError::Fetch {
            status,
            stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
          }
        })
      },
    )
    .and_then(|stdout| String::from_utf8(stdout).map_err(Into::into))
}

/// Rewrites the runner labels in place and reports each advance as a bump.
fn apply_runner_bumps(
  files: &[WorkflowFile],
  bumps: &[RunnerBump],
) -> Result<Vec<AppliedBump>, DependencyBumpError> {
  files
    .iter()
    .try_for_each(|file| rewrite_workflow(file, bumps))
    .map(|()| bumps.iter().map(applied_runner_bump).collect())
}

/// Writes the workflow back with its runner labels advanced.  A file no
/// advance touches is left unwritten.
fn rewrite_workflow(
  file: &WorkflowFile,
  bumps: &[RunnerBump],
) -> Result<(), DependencyBumpError> {
  let rewritten = bumps
    .iter()
    .fold(file.text.clone(), |text, bump| runners::rewrite(&text, bump));
  if rewritten == file.text {
    Ok(())
  } else {
    tracing::info!(
      path = %file.path.display(),
      "advancing the runner labels"
    );
    std::fs::write(&file.path, &rewritten).map_err(|source| {
      DependencyBumpError::WorkflowRewriteError {
        path: file.path.clone(),
        source,
      }
    })
  }
}

fn applied_runner_bump(bump: &RunnerBump) -> AppliedBump {
  AppliedBump {
    name: bump.variant.to_string(),
    from: runners::release(bump.from),
    to: runners::release(bump.to),
    entry: compose::runner_entry(bump),
  }
}

/// The post-update lockfile diff is the report, so the lockfiles must
/// start clean or the report would claim someone else's changes.
fn require_clean_lockfile(
  workspace_dir: &Path,
) -> Result<(), DependencyBumpError> {
  Command::new("git")
    .args([
      "diff",
      "--quiet",
      "--",
      "Cargo.lock",
      ":(glob)**/Cargo.lock",
    ])
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::LockfileCleanCheckError { source })
    .and_then(|status| match status.code() {
      Some(0) => Ok(()),
      Some(1) => Err(DependencyBumpError::LockfileDirtyError),
      _ => Err(DependencyBumpError::LockfileCleanCheckFailedError { status }),
    })
}

/// Runs `cargo update` across the whole workspace, streaming cargo's own
/// narration through to the user.  With `dry_run`, cargo previews and
/// touches nothing.
fn cargo_update(
  workspace_dir: &Path,
  dry_run: bool,
) -> Result<(), DependencyBumpError> {
  Command::new("cargo")
    .arg("update")
    .args(dry_run.then_some("--dry-run"))
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::CargoUpdateSpawnError { source })
    .and_then(|status| {
      status
        .success()
        .then_some(())
        .ok_or(DependencyBumpError::CargoUpdateFailedError { status })
    })
}

/// Re-pins every held package that `cargo update` advanced back to its
/// pre-update version.  A hold that cargo cannot re-pin fails the run
/// loudly — silently shipping a held bump is the one outcome the hold
/// table exists to prevent.
fn repin_held(
  workspace_dir: &Path,
  held: &[Hold],
  before: &Snapshot,
  lockfile_path: &Path,
) -> Result<(), DependencyBumpError> {
  held.iter().try_for_each(|hold| {
    lockfile::snapshot(lockfile_path).and_then(|current| {
      lockfile::bumps_between(before, &current)
        .into_iter()
        .filter(|bump| bump.name == hold.package)
        .try_for_each(|bump| repin(workspace_dir, &bump, &hold.reason))
    })
  })
}

/// Puts one held package back: `name@new` names the moved instance
/// unambiguously even when several versions coexist in the graph, and
/// `--precise old` restores exactly the pre-update version.
fn repin(
  workspace_dir: &Path,
  bump: &Bump,
  reason: &str,
) -> Result<(), DependencyBumpError> {
  tracing::info!(
    package = %bump.name,
    from = %bump.from,
    to = %bump.to,
    reason,
    "holding package: re-pinning to its pre-update version"
  );
  Command::new("cargo")
    .arg("update")
    .arg("--package")
    .arg(format!("{}@{}", bump.name, bump.to))
    .arg("--precise")
    .arg(&bump.from)
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::HoldRepinSpawnError {
      package: bump.name.clone(),
      source,
    })
    .and_then(|status| {
      status.success().then_some(()).ok_or(
        DependencyBumpError::HoldRepinFailedError {
          package: bump.name.clone(),
          status,
        },
      )
    })
}

/// Inserts one entry per bump and then normalises the file the way a local
/// pre-commit treefmt would (org-fmt wraps long lines), so the composed
/// changelog does not churn a later commit's diff.  A workspace without
/// the changelog file skips composition — the bump commit stands without
/// it.
fn update_changelog(
  workspace_dir: &Path,
  changelog_file: &str,
  bumps: &[AppliedBump],
) -> Result<bool, DependencyBumpError> {
  if !workspace_dir.join(changelog_file).is_file() {
    tracing::warn!(
      changelog = changelog_file,
      "no changelog file here; skipping the changelog entries"
    );
    return Ok(false);
  }
  bumps
    .iter()
    .try_for_each(|bump| insert_entry(workspace_dir, changelog_file, bump))
    .and_then(|()| org_fmt(workspace_dir, changelog_file))
    .map(|()| true)
}

fn insert_entry(
  workspace_dir: &Path,
  changelog_file: &str,
  bump: &AppliedBump,
) -> Result<(), DependencyBumpError> {
  Command::new("changelog-roller")
    .arg("insert-item")
    .arg("--input-file")
    .arg(changelog_file)
    .arg("--heading")
    .arg(bump.entry.heading.to_string())
    .arg("--body")
    .arg(&bump.entry.body)
    .arg("--in-place")
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::ChangelogInsertSpawnError {
      package: bump.name.clone(),
      source,
    })
    .and_then(|status| {
      status.success().then_some(()).ok_or(
        DependencyBumpError::ChangelogInsertFailedError {
          package: bump.name.clone(),
          status,
        },
      )
    })
}

fn org_fmt(
  workspace_dir: &Path,
  changelog_file: &str,
) -> Result<(), DependencyBumpError> {
  let path = workspace_dir.join(changelog_file);
  Command::new("org-fmt")
    .arg("--in-place")
    .arg(changelog_file)
    .current_dir(workspace_dir)
    .status()
    .map_err(|source| DependencyBumpError::OrgFmtSpawnError {
      path: path.clone(),
      source,
    })
    .and_then(|status| {
      status
        .success()
        .then_some(())
        .ok_or(DependencyBumpError::OrgFmtFailedError { path, status })
    })
}

/// Writes the TSV report: `name<TAB>from<TAB>to<TAB>heading` per bump, an
/// empty file when nothing moved.  The report is the scheduled workflow's
/// only contract with this tool; stdout narrates for humans and is never
/// parsed.
fn write_report(
  path: &Path,
  bumps: &[AppliedBump],
) -> Result<(), DependencyBumpError> {
  std::fs::write(
    path,
    bumps
      .iter()
      .map(|bump| {
        format!(
          "{}\t{}\t{}\t{}\n",
          bump.name, bump.from, bump.to, bump.entry.heading
        )
      })
      .collect::<String>(),
  )
  .map_err(|source| DependencyBumpError::ReportWriteError {
    path: path.to_path_buf(),
    source,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::runners::Variant;
  use std::io::{Read, Write};
  use std::net::TcpListener;

  const GA_TABLE: &str = "\
| Image | Architecture | YAML Label | Included Software |
| Ubuntu 26.04 | x64 | `ubuntu-26.04` | [ubuntu-26.04] |
| Ubuntu 26.04 Arm64 | arm64 | `ubuntu-26.04-arm` | [ubuntu-26.04-arm64] |
| Ubuntu 24.04 | x64 | `ubuntu-latest` or `ubuntu-24.04` | [ubuntu-24.04] |
";

  /// Serves one canned HTTP response on a loopback port; the URL to fetch.
  fn serve_once(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
      let (mut stream, _) = listener.accept().unwrap();
      let mut request = [0u8; 4096];
      let request_bytes = stream.read(&mut request).unwrap();
      assert!(request_bytes > 0, "curl sent an empty request");
      write!(
        stream,
        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n\
         {body}",
        body.len()
      )
      .unwrap();
    });
    format!("http://127.0.0.1:{port}/README.md")
  }

  /// A loopback URL nothing listens on.
  fn dead_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/README.md")
  }

  fn workflow(text: &str) -> WorkflowFile {
    WorkflowFile {
      path: PathBuf::from("ci.yml"),
      text: text.to_string(),
    }
  }

  fn workspace_with_workflows(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join(".github").join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    files.iter().for_each(|(name, text)| {
      std::fs::write(workflows.join(name), text).unwrap();
    });
    dir
  }

  #[test]
  fn fetch_readme_returns_the_body() {
    assert_eq!(fetch_readme(&serve_once(GA_TABLE)).unwrap(), GA_TABLE);
  }

  #[test]
  fn a_refused_connection_is_a_fetch_error() {
    assert!(matches!(
      fetch_readme(&dead_url()),
      Err(RunnerImageProbeError::Fetch { .. })
    ));
  }

  #[test]
  fn workflow_files_is_empty_without_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    assert!(workflow_files(dir.path()).unwrap().is_empty());
  }

  #[test]
  fn workflow_files_reads_yaml_in_path_order() {
    let dir = workspace_with_workflows(&[
      ("b.yaml", "b"),
      ("a.yml", "a"),
      ("notes.txt", "n"),
    ]);
    let files = workflow_files(dir.path())
      .unwrap()
      .into_iter()
      .map(|file| {
        (
          file
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
          file.text,
        )
      })
      .collect::<Vec<_>>();
    assert_eq!(
      files,
      vec![
        ("a.yml".to_string(), "a".to_string()),
        ("b.yaml".to_string(), "b".to_string())
      ]
    );
  }

  #[test]
  fn runner_bumps_skip_the_fetch_when_nothing_is_pinned() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
      "http://127.0.0.1:{}/README.md",
      listener.local_addr().unwrap().port()
    );
    let files = [workflow("runs-on: ubuntu-latest\n")];
    assert!(runner_bumps_or_none(&files, &url).is_empty());
    assert_eq!(
      listener.accept().unwrap_err().kind(),
      std::io::ErrorKind::WouldBlock
    );
  }

  #[test]
  fn runner_bumps_degrade_to_none_when_the_fetch_fails() {
    let files = [workflow("runs-on: ubuntu-24.04\n")];
    assert!(runner_bumps_or_none(&files, &dead_url()).is_empty());
  }

  #[test]
  fn runner_bumps_follow_the_readme() {
    let files = [workflow(
      "runs-on: ubuntu-24.04\nrunner: ubuntu-24.04-arm\n",
    )];
    assert_eq!(
      runner_bumps_or_none(&files, &serve_once(GA_TABLE)),
      vec![
        RunnerBump {
          variant: Variant::X64,
          from: 24,
          to: 26
        },
        RunnerBump {
          variant: Variant::Arm,
          from: 24,
          to: 26
        }
      ]
    );
  }

  #[test]
  fn applying_runner_bumps_rewrites_only_what_moves() {
    let dir = workspace_with_workflows(&[
      (
        "ci.yml",
        "runs-on: ubuntu-24.04\n{\"runner\":\"ubuntu-24.04-arm\"}\n\
         [ubuntu-24.04-arm64]\n",
      ),
      ("other.yml", "uses: ./x\n"),
    ]);
    let files = workflow_files(dir.path()).unwrap();
    let applied = apply_runner_bumps(
      &files,
      &[RunnerBump {
        variant: Variant::X64,
        from: 24,
        to: 26,
      }],
    )
    .unwrap();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].name, "GitHub-hosted Ubuntu runner image");
    assert_eq!(applied[0].from, "24.04");
    assert_eq!(applied[0].to, "26.04");
    assert_eq!(
      std::fs::read_to_string(dir.path().join(".github/workflows/ci.yml"))
        .unwrap(),
      "runs-on: ubuntu-26.04\n{\"runner\":\"ubuntu-24.04-arm\"}\n\
       [ubuntu-24.04-arm64]\n"
    );
    assert_eq!(
      std::fs::read_to_string(dir.path().join(".github/workflows/other.yml"))
        .unwrap(),
      "uses: ./x\n"
    );
  }
}
