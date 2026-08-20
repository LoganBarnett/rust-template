use crate::config::Config;
use crate::decision;
use crate::error::AppError;
use crate::state::{SessionState, MAX_ROUNDS};
use std::path::Path;
use std::process::{Command, Stdio};
use tracing::{debug, warn};

/// How the clippy gate left the turn.
pub enum ClippyOutcome {
  /// The workspace is clean; the review gate decides.
  Clean,
  /// Warnings remain and the turn is blocked with them.
  Block(String),
  /// Warnings remain but the round cap is spent; the review gate decides.
  Released,
  /// No toolchain is reachable; the gate steps aside rather than wedge a
  /// machine that cannot run it.
  Skipped,
}

/// A command as a program and its arguments — the seam is a shell string run
/// through `bash -c`, the real invocations are argument vectors.  (`bash -c`
/// runs the string as a command; bash has no long-form spelling of the flag.)
struct ClippyCommand {
  program: &'static str,
  args: Vec<String>,
}

const CLIPPY_ARGS: [&str; 7] = [
  "clippy",
  "--workspace",
  "--all-targets",
  "--all-features",
  "--",
  "--deny",
  "warnings",
];

/// Deterministic clippy gate for Rust changes.
pub fn gate(
  config: &Config,
  session: &SessionState,
) -> Result<ClippyOutcome, AppError> {
  command(config)
    .map_or(Ok(ClippyOutcome::Skipped), |command| run(&command, session))
}

fn command(config: &Config) -> Option<ClippyCommand> {
  config
    .clippy_seam()
    .map(|seam| ClippyCommand {
      program: "bash",
      args: vec!["-c".to_string(), seam.to_string()],
    })
    .or_else(|| {
      on_path("cargo").then(|| ClippyCommand {
        program: "cargo",
        args: CLIPPY_ARGS.iter().map(ToString::to_string).collect(),
      })
    })
    .or_else(|| {
      on_path("nix").then(|| ClippyCommand {
        program: "nix",
        args: ["develop", "--command", "cargo"]
          .iter()
          .chain(CLIPPY_ARGS.iter())
          .map(ToString::to_string)
          .collect(),
      })
    })
}

/// Whether `program` resolves on the current PATH, as `command -v` would
/// answer it.
fn on_path(program: &str) -> bool {
  std::env::var_os("PATH").is_some_and(|path| {
    std::env::split_paths(&path)
      .any(|dir| Path::new(&dir).join(program).is_file())
  })
}

fn run(
  command: &ClippyCommand,
  session: &SessionState,
) -> Result<ClippyOutcome, AppError> {
  debug!(program = command.program, "running the clippy gate");
  let output = Command::new(command.program)
    .args(&command.args)
    .stdin(Stdio::null())
    .output()
    .map_err(|source| AppError::ClippyInvocation {
      command: format!("{} {}", command.program, command.args.join(" ")),
      source,
    })?;
  // Bound consecutive clippy blocks so a warning the assistant genuinely
  // cannot clear (e.g. pre-existing in a drifted repo) does not wedge the
  // session — the same escape valve MAX_ROUNDS gives the review gate.
  let count = session.clippy_rounds()? + 1;
  if output.status.success() {
    session.clear_clippy_rounds()?;
    Ok(ClippyOutcome::Clean)
  } else if count > MAX_ROUNDS {
    warn!("releasing after {MAX_ROUNDS} unresolved clippy rounds");
    session.clear_clippy_rounds()?;
    Ok(ClippyOutcome::Released)
  } else {
    session.write_clippy_rounds(count)?;
    Ok(ClippyOutcome::Block(decision::clippy_reason(&format!(
      "{}{}",
      String::from_utf8_lossy(&output.stdout),
      String::from_utf8_lossy(&output.stderr)
    ))))
  }
}
