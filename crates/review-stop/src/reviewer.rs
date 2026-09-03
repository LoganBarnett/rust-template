use crate::config::Config;
use crate::error::{panic_detail, AppError};
use crate::path_lookup::on_path;
use crate::prompt::REVIEWER;
use crate::worktree;
use serde::{Deserialize, Serialize};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;
use tracing::debug;

/// Set in the nested reviewer's environment.  A gate that finds it releases
/// at once, so the reviewer's own Stop hook never reviews the reviewer.  The
/// agent under review cannot reach the hook's environment from its tools, so
/// it cannot set this to skip its own review.
pub const NESTED_ENV: &str = "RUST_TEMPLATE_REVIEW_NESTED";

/// The verdict's shape, enforced by the CLI's structured output.
const SCHEMA: &str = r#"{"type":"object","properties":{"findings":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"line":{"type":"integer"},"convention":{"type":"string"},"document":{"type":"string"},"fix":{"type":"string"}},"required":["path","line","convention","document","fix"]}}},"required":["findings"]}"#;

/// The convention documents, in the import order `llms.org` declares (README
/// first, framing what the project is).  The template repository also carries
/// the emitted conventions under `template/`; a spawn has the first three only,
/// and a path absent at `HEAD` is skipped.
const CONVENTION_PATHS: [&str; 6] = [
  "README.org",
  "CONTRIBUTING.org",
  "llms.org",
  "template/README.org",
  "template/CONTRIBUTING.org",
  "template/llms.org",
];

/// The reviewer may look but not touch.
const READ_ONLY_TOOLS: &str = "Read,Grep,Glob";

/// A top-level licence is boilerplate the conventions do not govern.
const LICENSE: &str = "LICENSE";

/// How much of an unreadable reviewer output to carry in the error.
const EXCERPT_CHARS: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
  pub path: String,
  pub line: u64,
  pub convention: String,
  pub document: String,
  pub fix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
  pub findings: Vec<Finding>,
}

impl Verdict {
  pub fn passes(&self) -> bool {
    self.findings.is_empty()
  }
}

/// Everything the reviewer is allowed to know, assembled here so nothing about
/// the scope is left to it or to the agent under review.  `head` is the commit
/// the conventions and the diff base come from; on an unborn branch the
/// working copies and the empty tree stand in, since there is no committed
/// rule an edit could have softened.
pub fn packet(head: Option<&str>) -> Result<String, AppError> {
  let base =
    head.map_or_else(worktree::empty_tree, |commit| Ok(commit.to_string()))?;
  Ok(
    [
      "REVIEW PACKET\n=============\n\nAssembled by \
       rust-template-review-stop.  It is the complete scope of the review: \
       every hunk of the diff and every untracked file below must be judged \
       against the conventions below.\n\n"
        .to_string(),
      format!(
        "CONVENTIONS (as committed at {})\n{}\n\n",
        head.unwrap_or("the unborn branch's working tree"),
        "-".repeat(40)
      ),
      conventions(head)?,
      global_instructions()?,
      format!(
        "CHANGES UNDER REVIEW\n--------------------\n\n{}",
        block(
          &format!("DIFF (git diff {base})"),
          &worktree::diff_against(&base)?
        )
      ),
      untracked_sections()?,
      "Review every change above against the conventions above and report \
       through the structured output.\n"
        .to_string(),
    ]
    .concat(),
  )
}

fn block(label: &str, body: &str) -> String {
  format!(
    "----- BEGIN {label} -----\n{}\n----- END {label} -----\n\n",
    body.trim_end_matches('\n')
  )
}

fn conventions(head: Option<&str>) -> Result<String, AppError> {
  CONVENTION_PATHS
    .iter()
    .map(|path| {
      convention(head, path).map(|document| {
        document.map(|text| block(&format!("CONVENTIONS: {path}"), &text))
      })
    })
    .collect::<Result<Vec<_>, _>>()
    .map(|blocks| blocks.into_iter().flatten().collect())
}

fn convention(
  head: Option<&str>,
  path: &str,
) -> Result<Option<String>, AppError> {
  head.map_or_else(|| working_copy(path), |_| worktree::committed_file(path))
}

fn working_copy(path: &str) -> Result<Option<String>, AppError> {
  match std::fs::read_to_string(path) {
    Ok(text) => Ok(Some(text)),
    Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
    Err(source) => Err(AppError::ConventionRead {
      path: PathBuf::from(path),
      source,
    }),
  }
}

/// The user's global instructions, which the conventions defer to on matters
/// like prose spacing.  They live outside the repository, so they are read
/// from disk; a user without the file simply contributes nothing.
fn global_instructions() -> Result<String, AppError> {
  std::env::var_os("HOME")
    .map(|home| PathBuf::from(home).join(".claude").join("CLAUDE.md"))
    .map_or(Ok(String::new()), |path| match std::fs::read_to_string(&path) {
      Ok(text) => Ok(format!(
        "GLOBAL INSTRUCTIONS\n-------------------\n\n{}",
        block(&format!("GLOBAL INSTRUCTIONS: {}", path.display()), &text)
      )),
      Err(error) if error.kind() == ErrorKind::NotFound => Ok(String::new()),
      Err(source) => Err(AppError::GlobalInstructionsRead { path, source }),
    })
}

/// The diff cannot show an untracked file, so each one travels whole.
fn untracked_sections() -> Result<String, AppError> {
  worktree::untracked_files()?
    .into_iter()
    .filter(|path| path != LICENSE)
    .map(|path| untracked_block(&path))
    .collect()
}

fn untracked_block(path: &str) -> Result<String, AppError> {
  std::fs::read(path)
    .map_err(|source| AppError::UntrackedFileRead {
      path: PathBuf::from(path),
      source,
    })
    .map(|bytes| {
      block(
        &format!("UNTRACKED FILE: {path}"),
        &String::from_utf8(bytes).unwrap_or_else(|error| {
          format!(
            "(binary content, {} bytes; not shown)",
            error.as_bytes().len()
          )
        }),
      )
    })
}

/// Run the reviewer over the packet and read back its verdict.  A non-zero
/// exit, an error envelope, or output that is not a verdict are all failures;
/// none of them is a pass.
pub fn review(config: &Config, packet: &str) -> Result<Verdict, AppError> {
  let prompt = prompt_file()?;
  let mut command = command(config, prompt.path())?;
  let description = format!("{command:?}");
  debug!(command = %description, "running the reviewer");
  let invocation = |source| AppError::ReviewerInvocation {
    command: description.clone(),
    source,
  };
  let mut child = command
    .env(NESTED_ENV, "1")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(&invocation)?;
  let pipe_broke = feed(child.stdin.take(), packet).map_err(&invocation)?;
  let finished = wait_with_deadline(
    child,
    Duration::from_secs(config.reviewer_timeout_secs),
    &invocation,
  )?;
  if finished.timed_out {
    Err(AppError::ReviewerTimedOut {
      secs: config.reviewer_timeout_secs,
    })
  } else if !finished.status.success() {
    Err(AppError::ReviewerFailed {
      status: finished.status.to_string(),
      stderr: String::from_utf8_lossy(&finished.stderr).trim().to_string(),
    })
  } else if pipe_broke {
    Err(AppError::ReviewerIgnoredPacket)
  } else {
    verdict(&String::from_utf8_lossy(&finished.stdout))
  }
}

/// How long between checks of whether the reviewer has exited.
const POLL: Duration = Duration::from_millis(200);

/// The reviewer's outcome once it has finished or been stopped.
struct Finished {
  status: ExitStatus,
  stdout: Vec<u8>,
  stderr: Vec<u8>,
  timed_out: bool,
}

/// Wait for the reviewer up to `limit`, draining its stdout and stderr on
/// their own threads so neither stream can fill its pipe and deadlock the
/// wait, and killing it once the deadline passes.  A kill is reported as
/// `timed_out` rather than through the exit status, since the status of a
/// killed process says nothing useful.
fn wait_with_deadline(
  mut child: Child,
  limit: Duration,
  invocation: &impl Fn(std::io::Error) -> AppError,
) -> Result<Finished, AppError> {
  let stdout = drain(child.stdout.take());
  let stderr = drain(child.stderr.take());
  let deadline = Instant::now() + limit;
  loop {
    if let Some(status) = child.try_wait().map_err(invocation)? {
      break Ok(Finished {
        status,
        stdout: join_drain(stdout, "stdout", invocation)?,
        stderr: join_drain(stderr, "stderr", invocation)?,
        timed_out: false,
      });
    } else if Instant::now() >= deadline {
      child.kill().map_err(invocation)?;
      let status = child.wait().map_err(invocation)?;
      // A timed-out reviewer's partial output is not needed, and a killed
      // process may leave descendants holding the pipes open, which would
      // block a join.  Leave the drain threads to finish on their own (they
      // die with this short-lived gate process) rather than wait on them.
      drop((stdout, stderr));
      break Ok(Finished {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
        timed_out: true,
      });
    }
    thread::sleep(POLL);
  }
}

/// Read a child stream to the end on its own thread.
fn drain(
  pipe: Option<impl Read + Send + 'static>,
) -> JoinHandle<std::io::Result<Vec<u8>>> {
  thread::spawn(move || {
    pipe.map_or(Ok(Vec::new()), |mut stream| {
      let mut buffer = Vec::new();
      stream.read_to_end(&mut buffer).map(|_| buffer)
    })
  })
}

/// Collect a drain thread's bytes, distinguishing a panicked thread from a
/// read error so neither is swallowed.
fn join_drain(
  handle: JoinHandle<std::io::Result<Vec<u8>>>,
  stream: &'static str,
  invocation: &impl Fn(std::io::Error) -> AppError,
) -> Result<Vec<u8>, AppError> {
  handle
    .join()
    .map_err(|payload| AppError::ReviewerOutputThreadPanicked {
      stream,
      detail: panic_detail(payload.as_ref()),
    })?
    .map_err(invocation)
}

/// Write the packet to the reviewer's stdin and close it, reporting whether the
/// reviewer closed its end before the packet was fully written (a broken
/// pipe).  A broken pipe is not itself an error to return — `review` weighs it
/// against the exit status, since a reviewer that failed will also have broken
/// the pipe — but a reviewer that exits *successfully* without draining the
/// packet never saw what it was meant to judge, which `review` rejects.
fn feed(stdin: Option<ChildStdin>, packet: &str) -> std::io::Result<bool> {
  stdin.map_or(Ok(false), |mut stdin| {
    match stdin.write_all(packet.as_bytes()) {
      Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(true),
      Err(error) => Err(error),
      Ok(()) => Ok(false),
    }
  })
}

/// The prompt in a temporary file for `--append-system-prompt-file`, created in
/// the system temp directory so the error can name a location either way.
fn prompt_file() -> Result<NamedTempFile, AppError> {
  let dir = std::env::temp_dir();
  let mut file = tempfile::Builder::new()
    .prefix("review-stop-reviewer-")
    .suffix(".md")
    .tempfile_in(&dir)
    .map_err(|source| AppError::ReviewerPromptWrite {
      path: dir.clone(),
      source,
    })?;
  let path = file.path().to_path_buf();
  file
    .write_all(REVIEWER.as_bytes())
    .map(|()| file)
    .map_err(|source| AppError::ReviewerPromptWrite { path, source })
}

fn command(config: &Config, prompt: &Path) -> Result<Command, AppError> {
  config
    .reviewer_seam()
    .map_or_else(|| claude(config, prompt), |seam| Ok(shell(seam)))
}

/// The seam is a shell string.  (`bash -c` runs the string as a command; bash
/// has no long-form spelling of the flag.)
fn shell(seam: &str) -> Command {
  let mut command = Command::new("bash");
  command.args(["-c", seam]);
  command
}

/// The real reviewer: a headless `claude` run configured by the arguments
/// below, or `ReviewerNotOnPath` when the command is absent.
fn claude(config: &Config, prompt: &Path) -> Result<Command, AppError> {
  on_path("claude")
    .then(|| {
      let mut command = Command::new("claude");
      command
        .args([
          "--print",
          "--output-format",
          "json",
          "--json-schema",
          SCHEMA,
          "--append-system-prompt-file",
        ])
        .arg(prompt)
        .args([
          "--tools",
          READ_ONLY_TOOLS,
          "--allowedTools",
          READ_ONLY_TOOLS,
          "--no-session-persistence",
          "--max-turns",
        ])
        .arg(config.reviewer_max_turns.to_string());
      if let Some(model) = config.reviewer_model() {
        command.args(["--model", model]);
      }
      command
    })
    .ok_or(AppError::ReviewerNotOnPath)
}

/// The verdict inside the CLI's result envelope: `structured_output` carries
/// it parsed, and `result` carries the same JSON as text, which serves as the
/// fallback when only one of them is present.
fn verdict(stdout: &str) -> Result<Verdict, AppError> {
  let unparseable = |detail: String| AppError::VerdictUnparseable {
    detail,
    output: stdout.chars().take(EXCERPT_CHARS).collect(),
  };
  let envelope: serde_json::Value = serde_json::from_str(stdout.trim())
    .map_err(|error| unparseable(error.to_string()))?;
  if envelope
    .get("is_error")
    .and_then(serde_json::Value::as_bool)
    == Some(true)
  {
    Err(AppError::ReviewerReportedError {
      detail: envelope
        .get("result")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("no detail in the envelope")
        .to_string(),
    })
  } else {
    envelope
      .get("structured_output")
      .filter(|value| !value.is_null())
      .map(|value| {
        serde_json::from_value::<Verdict>(value.clone())
          .map_err(|error| error.to_string())
      })
      .or_else(|| {
        envelope
          .get("result")
          .and_then(serde_json::Value::as_str)
          .map(|text| {
            serde_json::from_str::<Verdict>(text)
              .map_err(|error| error.to_string())
          })
      })
      .ok_or_else(|| {
        unparseable(
          "the envelope carries neither structured_output nor result"
            .to_string(),
        )
      })?
      .map_err(unparseable)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn structured_output_is_preferred() {
    let parsed = verdict(
      r#"{"is_error":false,"result":"garbage","structured_output":{"findings":[]}}"#,
    )
    .unwrap();
    assert!(parsed.passes());
  }

  #[test]
  fn result_text_is_the_fallback() {
    let parsed = verdict(
      r#"{"is_error":false,"result":"{\"findings\":[{\"path\":\"a\",\"line\":1,\"convention\":\"c\",\"document\":\"d\",\"fix\":\"f\"}]}"}"#,
    )
    .unwrap();
    assert_eq!(parsed.findings.len(), 1);
  }

  #[test]
  fn an_error_envelope_is_not_a_verdict() {
    assert!(matches!(
      verdict(r#"{"is_error":true,"result":"rate limited"}"#),
      Err(AppError::ReviewerReportedError { detail }) if detail == "rate limited"
    ));
  }

  #[test]
  fn non_json_output_is_unparseable() {
    assert!(matches!(
      verdict("not json"),
      Err(AppError::VerdictUnparseable { .. })
    ));
  }

  #[test]
  fn blocks_are_delimited_and_normalised() {
    assert_eq!(
      block("X", "body\n\n"),
      "----- BEGIN X -----\nbody\n----- END X -----\n\n"
    );
  }
}
