//! The reviewer: what it is told, and how its verdict is read back.
//!
//! The packet is assembled here rather than left to the reviewer to gather,
//! so nothing about the scope depends on what the tree under review says.

use crate::error::{panic_detail, ReviewError};
use crate::markup::Markup;
use crate::patch;
use crate::prompt::REVIEWER;
use crate::worktree::Worktree;
use gix::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;
use tracing::{debug, info};

/// Set in the nested reviewer's environment.  A Stop-hook gate that finds it
/// releases at once, so a review this tool runs never triggers a review of its
/// own.
pub const NESTED_ENV: &str = "RUST_TEMPLATE_REVIEW_NESTED";

/// The verdict's shape, enforced by the CLI's structured output.
const SCHEMA: &str = r#"{"type":"object","properties":{"findings":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"line":{"type":"integer"},"convention":{"type":"string"},"document":{"type":"string"},"fix":{"type":"string"}},"required":["path","line","convention","document","fix"]}}},"required":["findings"]}"#;

/// The convention documents, README first so what the project is frames the
/// rest, with the template's emitted copies last.  A spawn has the first three
/// only; a path absent at the diff base is skipped.
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

/// How the nested reviewer is run.
#[derive(Debug, Clone)]
pub struct Options {
  pub model: Option<String>,
  pub max_turns: u32,
  pub timeout_secs: u64,
  /// The markup the reviewer is told to write its prose in.
  pub markup: crate::markup::Markup,
  /// A shell command run in place of the real reviewer, for tests.  It is fed
  /// the packet on stdin and must print the same JSON envelope `claude --print
  /// --output-format json` does.
  pub command: Option<String>,
}

/// Everything the reviewer is allowed to know.
pub struct Scope<'a> {
  /// The repository, read for the conventions, the diff, and the untracked
  /// files regardless of the working directory the tool was run from.
  pub worktree: &'a Worktree,
  /// The object the conventions and the diff come from.
  pub base: &'a ObjectId,
  /// The paths this round judges; the diff is restricted to them.
  pub stale: &'a [String],
  /// Where a path under review was renamed from, keyed by the path.
  pub origins: &'a BTreeMap<String, String>,
  /// What earlier rounds reported, pre-rendered by the record.
  pub history: &'a str,
}

pub fn packet(scope: &Scope) -> Result<String, ReviewError> {
  Ok(
    [
      "REVIEW PACKET\n=============\n\nAssembled by rust-template-review.  \
       It is the complete scope of the review: every hunk of the diff and \
       every untracked file below must be judged against the conventions \
       below.  Files judged on an earlier round and unchanged since are \
       deliberately absent.\n\n"
        .to_string(),
      format!(
        "CONVENTIONS (as committed at {})\n{}\n\n",
        scope.base,
        "-".repeat(40)
      ),
      conventions(scope.worktree, scope.base)?,
      global_instructions()?,
      history_section(scope.history),
      format!(
        "CHANGES UNDER REVIEW\n--------------------\n\n{}",
        block(
          &format!("DIFF (against {})", scope.base),
          &patch::render(
            scope.worktree,
            scope.base,
            scope.stale,
            scope.origins
          )?
        )
      ),
      untracked_sections(scope.worktree, scope.stale)?,
      "Review every change above against the conventions above and report \
       through the structured output.\n"
        .to_string(),
    ]
    .concat(),
  )
}

fn history_section(history: &str) -> String {
  if history.is_empty() {
    String::new()
  } else {
    format!(
      "REVIEW HISTORY\n--------------\n\n{}",
      block("REVIEW HISTORY", history)
    )
  }
}

fn block(label: &str, body: &str) -> String {
  format!(
    "----- BEGIN {label} -----\n{}\n----- END {label} -----\n\n",
    body.trim_end_matches('\n')
  )
}

fn conventions(
  worktree: &Worktree,
  base: &ObjectId,
) -> Result<String, ReviewError> {
  CONVENTION_PATHS
    .iter()
    .map(|path| {
      worktree.file_at(base, path).map(|document| {
        document.map(|text| block(&format!("CONVENTIONS: {path}"), &text))
      })
    })
    .collect::<Result<Vec<_>, _>>()
    .map(|blocks| blocks.into_iter().flatten().collect())
}

/// The user's global instructions, which the conventions defer to on matters
/// like prose spacing.  They live outside the repository, so they are read
/// from disk; a user without the file simply contributes nothing.
fn global_instructions() -> Result<String, ReviewError> {
  std::env::var_os("HOME")
    .map(|home| PathBuf::from(home).join(".claude").join("CLAUDE.md"))
    .map_or(Ok(String::new()), |path| match std::fs::read_to_string(&path) {
      Ok(text) => Ok(format!(
        "GLOBAL INSTRUCTIONS\n-------------------\n\n{}",
        block(&format!("GLOBAL INSTRUCTIONS: {}", path.display()), &text)
      )),
      Err(error) if error.kind() == ErrorKind::NotFound => Ok(String::new()),
      Err(source) => Err(ReviewError::GlobalInstructionsRead { path, source }),
    })
}

/// The diff cannot show an untracked file, so each one this round judges
/// travels whole.
fn untracked_sections(
  worktree: &Worktree,
  stale: &[String],
) -> Result<String, ReviewError> {
  worktree
    .untracked_files()?
    .into_iter()
    .filter(|path| path != LICENSE && stale.contains(path))
    .map(|path| untracked_block(worktree.root(), &path))
    .collect()
}

fn untracked_block(root: &Path, path: &str) -> Result<String, ReviewError> {
  std::fs::read(root.join(path))
    .map_err(|source| ReviewError::UntrackedFileRead {
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

/// Run the reviewer over the packet and read back its verdict.  Anything
/// short of a verdict is a failure, never a pass.
pub fn review(options: &Options, packet: &str) -> Result<Verdict, ReviewError> {
  let prompt = prompt_file(options.markup)?;
  let mut command = command(options, prompt.path())?;
  let description = format!("{command:?}");
  debug!(command = %description, "running the reviewer");
  info!(
    model = options.model.as_deref().unwrap_or("the CLI default"),
    packet_bytes = packet.len(),
    timeout_secs = options.timeout_secs,
    "handing the changes to the reviewer"
  );
  let invocation = |source| ReviewError::ReviewerInvocation {
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
    Duration::from_secs(options.timeout_secs),
    &invocation,
  )?;
  if finished.timed_out {
    Err(ReviewError::ReviewerTimedOut {
      secs: options.timeout_secs,
    })
  } else if !finished.status.success() {
    Err(ReviewError::ReviewerFailed {
      status: finished.status.to_string(),
      stderr: String::from_utf8_lossy(&finished.stderr).trim().to_string(),
    })
  } else if pipe_broke {
    Err(ReviewError::ReviewerIgnoredPacket)
  } else {
    verdict(&String::from_utf8_lossy(&finished.stdout))
  }
}

/// How long between checks of whether the reviewer has exited.
const POLL: Duration = Duration::from_millis(200);

/// How often the wait reports that the reviewer is still running.  Frequent
/// enough that a run never looks hung, rare enough not to bury the findings.
const HEARTBEAT: Duration = Duration::from_secs(15);

struct Finished {
  status: ExitStatus,
  stdout: Vec<u8>,
  stderr: Vec<u8>,
  timed_out: bool,
}

/// Wait for the reviewer up to `limit`, draining its stdout and stderr on
/// their own threads so neither can fill its pipe and deadlock the wait, and
/// killing it once the deadline passes.  A kill is reported as `timed_out`
/// rather than through the exit status, since the status of a killed process
/// says nothing useful.
fn wait_with_deadline(
  mut child: Child,
  limit: Duration,
  invocation: &impl Fn(std::io::Error) -> ReviewError,
) -> Result<Finished, ReviewError> {
  let received = Arc::new(AtomicUsize::new(0));
  let stdout = drain(child.stdout.take(), Arc::clone(&received));
  let stderr = drain(child.stderr.take(), Arc::clone(&received));
  let started = Instant::now();
  let deadline = started + limit;
  let mut beat = started;
  loop {
    if let Some(status) = child.try_wait().map_err(invocation)? {
      info!(
        seconds = started.elapsed().as_secs(),
        bytes = received.load(Ordering::Relaxed),
        "the reviewer finished"
      );
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
      // block a join.  Leave the drain threads to finish on their own rather
      // than wait on them.
      drop((stdout, stderr));
      break Ok(Finished {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
        timed_out: true,
      });
    }
    if beat.elapsed() >= HEARTBEAT {
      beat = Instant::now();
      // A review is minutes of silence otherwise, and silence reads the same
      // whether the reviewer is thinking or wedged.  The byte count stays zero
      // for most of a run because the CLI buffers its JSON to the end, so this
      // reports that the process is alive rather than that a model is
      // answering.
      info!(
        seconds = started.elapsed().as_secs(),
        of = limit.as_secs(),
        bytes = received.load(Ordering::Relaxed),
        "still waiting for the reviewer"
      );
    }
    thread::sleep(POLL);
  }
}

/// A reader that reports every byte it yields to a shared counter, so the
/// wait loop can see progress on a stream another thread owns.
struct Counted<R> {
  inner: R,
  received: Arc<AtomicUsize>,
}

impl<R: Read> Read for Counted<R> {
  fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
    self.inner.read(buf).inspect(|&read| {
      self.received.fetch_add(read, Ordering::Relaxed);
    })
  }
}

/// Read a child stream to its end on its own thread, counting the bytes as
/// they arrive so the wait loop can report whether anything is coming back.
fn drain(
  pipe: Option<impl Read + Send + 'static>,
  received: Arc<AtomicUsize>,
) -> JoinHandle<std::io::Result<Vec<u8>>> {
  thread::spawn(move || {
    pipe.map_or(Ok(Vec::new()), |stream| {
      let mut buffer = Vec::new();
      Counted {
        inner: stream,
        received,
      }
      .read_to_end(&mut buffer)
      .map(|_| buffer)
    })
  })
}

/// Collect a drain thread's bytes, distinguishing a panicked thread from a
/// read error so neither is swallowed.
fn join_drain(
  handle: JoinHandle<std::io::Result<Vec<u8>>>,
  stream: &'static str,
  invocation: &impl Fn(std::io::Error) -> ReviewError,
) -> Result<Vec<u8>, ReviewError> {
  handle
    .join()
    .map_err(|payload| ReviewError::ReviewerOutputThreadPanicked {
      stream,
      detail: panic_detail(payload.as_ref()),
    })?
    .map_err(invocation)
}

/// Write the packet to the reviewer's stdin and close it, reporting whether
/// the reviewer closed its end before the packet was fully written.  A broken
/// pipe is not itself an error to return — `review` weighs it against the exit
/// status, since a reviewer that failed will also have broken the pipe — but a
/// reviewer that exits *successfully* without draining the packet never saw
/// what it was meant to judge.
///
/// This catches only a packet larger than the pipe buffer.  A small one is
/// written whole into the buffer and reports success even if nothing ever
/// reads it, so the check cannot be relied on for a short change set.
fn feed(stdin: Option<ChildStdin>, packet: &str) -> std::io::Result<bool> {
  stdin.map_or(Ok(false), |mut stdin| {
    match stdin.write_all(packet.as_bytes()) {
      Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(true),
      Err(error) => Err(error),
      Ok(()) => Ok(false),
    }
  })
}

/// The prompt in a temporary file for `--append-system-prompt-file`, created
/// in the system temp directory so the error can name a location either way.
fn prompt_file(markup: Markup) -> Result<NamedTempFile, ReviewError> {
  let dir = std::env::temp_dir();
  let mut file = tempfile::Builder::new()
    .prefix("review-reviewer-")
    .suffix(".md")
    .tempfile_in(&dir)
    .map_err(|source| ReviewError::ReviewerPromptWrite {
      path: dir.clone(),
      source,
    })?;
  let path = file.path().to_path_buf();
  file
    .write_all(format!("{REVIEWER}{}", markup.instruction()).as_bytes())
    .map(|()| file)
    .map_err(|source| ReviewError::ReviewerPromptWrite { path, source })
}

fn command(options: &Options, prompt: &Path) -> Result<Command, ReviewError> {
  options
    .command
    .as_deref()
    .map_or_else(|| claude(options, prompt), |seam| Ok(shell(seam)))
}

/// The seam is a shell string.  (`bash -c` runs the string as a command; bash
/// has no long-form spelling of the flag.)
fn shell(seam: &str) -> Command {
  let mut command = Command::new("bash");
  command.args(["-c", seam]);
  command
}

/// The real reviewer: a headless `claude` run, or `ReviewerNotOnPath` when the
/// command is absent.
fn claude(options: &Options, prompt: &Path) -> Result<Command, ReviewError> {
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
        .arg(options.max_turns.to_string());
      if let Some(model) = options.model.as_deref() {
        command.args(["--model", model]);
      }
      command
    })
    .ok_or(ReviewError::ReviewerNotOnPath)
}

/// Whether `program` resolves on `PATH`.
fn on_path(program: &str) -> bool {
  std::env::var_os("PATH").is_some_and(|paths| {
    std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
  })
}

/// The verdict inside the CLI's result envelope: `structured_output` carries
/// it parsed, and `result` carries the same JSON as text, which serves as the
/// fallback when only one of them is present.
fn verdict(stdout: &str) -> Result<Verdict, ReviewError> {
  let excerpt = || -> String { stdout.chars().take(EXCERPT_CHARS).collect() };
  let envelope: serde_json::Value = serde_json::from_str(stdout.trim())
    .map_err(|source| ReviewError::VerdictUnparseable {
      excerpt: excerpt(),
      source,
    })?;
  if envelope
    .get("is_error")
    .and_then(serde_json::Value::as_bool)
    == Some(true)
  {
    Err(ReviewError::ReviewerReportedError {
      message: envelope
        .get("result")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("no detail in the envelope")
        .to_string(),
    })
  } else {
    reported(&envelope).map_or_else(
      || Err(ReviewError::VerdictMissing { excerpt: excerpt() }),
      |parsed| {
        parsed.map_err(|source| ReviewError::VerdictUnparseable {
          excerpt: excerpt(),
          source,
        })
      },
    )
  }
}

/// The verdict from whichever envelope field carries it.  `None` means the
/// envelope carried neither field, which is a different failure from one
/// carrying a malformed verdict and reads differently to whoever has to fix
/// it — so the parse error travels back rather than being discarded.
fn reported(
  envelope: &serde_json::Value,
) -> Option<Result<Verdict, serde_json::Error>> {
  let as_text = || {
    envelope
      .get("result")
      .and_then(serde_json::Value::as_str)
      .map(serde_json::from_str::<Verdict>)
  };
  match envelope
    .get("structured_output")
    .filter(|value| !value.is_null())
    .map(|value| serde_json::from_value::<Verdict>(value.clone()))
  {
    // A malformed structured field still lets the text field answer, since
    // the CLI may populate only one of the two meaningfully.
    Some(Err(malformed)) => as_text().or(Some(Err(malformed))),
    parsed => parsed.or_else(as_text),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_structured_verdict_is_read() {
    let verdict =
      verdict(r#"{"is_error":false,"structured_output":{"findings":[]}}"#);
    assert!(verdict.is_ok_and(|found| found.findings.is_empty()));
  }

  #[test]
  fn a_result_string_verdict_is_read() {
    let verdict = verdict(r#"{"is_error":false,"result":"{\"findings\":[]}"}"#);
    assert!(verdict.is_ok());
  }

  #[test]
  fn an_error_envelope_is_never_a_pass() {
    assert!(matches!(
      verdict(r#"{"is_error":true,"result":"context exhausted"}"#),
      Err(ReviewError::ReviewerReportedError { .. }),
    ));
  }

  #[test]
  fn an_envelope_carrying_no_verdict_says_which_failure_it_is() {
    assert!(matches!(
      verdict(r#"{"is_error":false}"#),
      Err(ReviewError::VerdictMissing { .. }),
    ));
  }

  #[test]
  fn a_malformed_verdict_keeps_the_parse_error() {
    assert!(matches!(
      verdict(r#"{"is_error":false,"structured_output":{"nope":1}}"#),
      Err(ReviewError::VerdictUnparseable { .. }),
    ));
  }

  #[test]
  fn output_that_is_not_a_verdict_is_never_a_pass() {
    assert!(matches!(
      verdict("I could not complete the review."),
      Err(ReviewError::VerdictUnparseable { .. }),
    ));
  }

  #[test]
  fn the_history_section_is_omitted_when_empty() {
    assert!(history_section("").is_empty());
    assert!(history_section("Round 1").contains("REVIEW HISTORY"));
  }
}
