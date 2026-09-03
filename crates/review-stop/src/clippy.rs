use crate::config::Config;
use crate::decision;
use crate::error::AppError;
use crate::path_lookup::on_path;
use std::process::{Command, Stdio};
use tracing::debug;

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

/// Deterministic clippy gate for Rust changes: the block reason when the
/// workspace does not pass, `None` when it does.  With no toolchain reachable
/// the gate fails closed — it cannot vouch for the Rust changes, so it errors
/// rather than let them reach the review unchecked; the caller turns that into
/// a block the turn cannot end through.
pub fn block_reason(config: &Config) -> Result<Option<String>, AppError> {
  command(config)
    .map_or_else(|| Err(AppError::ClippyUnavailable), |command| run(&command))
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

fn run(command: &ClippyCommand) -> Result<Option<String>, AppError> {
  debug!(program = command.program, "running the clippy gate");
  Command::new(command.program)
    .args(&command.args)
    .stdin(Stdio::null())
    .output()
    .map_err(|source| AppError::ClippyInvocation {
      command: format!("{} {}", command.program, command.args.join(" ")),
      source,
    })
    .map(|output| {
      (!output.status.success()).then(|| {
        decision::clippy_reason(&format!(
          "{}{}",
          String::from_utf8_lossy(&output.stdout),
          String::from_utf8_lossy(&output.stderr)
        ))
      })
    })
}
