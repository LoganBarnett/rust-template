//! Black-box tests for the code-review Stop hook's gate.
//!
//! The gate inspects the git working tree and the session transcript, runs a
//! clippy gate when Rust files changed, then either releases the turn or emits
//! a block decision.  Every input is injectable, so the cases drive every
//! branch deterministically without a real working tree, a real compile, or a
//! live Claude session.  The gate keeps its per-session state under `TMPDIR`,
//! which is pointed at a scratch directory per test, so cases that span several
//! Stops share state only with themselves.
//!
//! Each case feeds a crafted transcript and file list, then asserts whether
//! the gate releases (no stdout) or blocks (a `{"decision":"block"}` JSON
//! document).  The load-bearing cases are the delivery shapes a real harness
//! hands a verdict back on.
//!
//! The binary is invoked from this crate's directory, inside the repository,
//! so its git work-tree probe succeeds the way it does in a real session; the
//! fixture seams replace every other git read it would make.

// clippy's in-test heuristic does not cover free helper fns in an integration
// test binary, so the exemption is stated at file level.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

/// The gate binary under test, resolved by cargo for this crate's tests.
const GATE: &str = env!("CARGO_BIN_EXE_rust-template-review-stop");

// ── Fixtures ─────────────────────────────────────────────────────────

// Working-tree file lists.  Every changed file is gated, prose included; Rust
// files also arm the clippy gate.
const FILES_PROSE: &str = "README.org\ndocs/notes.txt\n";
const FILES_CODE: &str = "src/main.rs\n";
// The same change plus one more file: a different working tree from
// FILES_CODE, so the fingerprint the gate stores on release no longer matches.
const FILES_CODE_TWO: &str = "src/main.rs\nsrc/lib.rs\n";
// A change that is not Rust: gated for review, but the clippy gate must leave
// it alone.
const FILES_CODE_NONRUST: &str = "flake.nix\n";
const FILES_NONE: &str = "";

// A transcript with a single user prompt and no review.
const TX_NOREVIEW: &str = r#"{"type":"user","message":{"content":"please change the code"}}
"#;

// A passing review recorded under the "Agent" tool name.
const TX_AGENT_PASS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
"#;

// The same passing review recorded under the stock "Task" tool name.
const TX_TASK_PASS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Task","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
"#;

// A review that reported findings rather than passing.
const TX_FINDINGS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: FINDINGS\nsomething is off"}]}]}}
"#;

// A passing review that predates a newer user prompt, so it must not count
// toward the current turn.
const TX_STALE_REVIEW: &str = r#"{"type":"user","message":{"content":"first prompt"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
{"type":"user","message":{"content":"second prompt with fresh changes"}}
"#;

// A background subagent: the tool_result for the spawn holds only launch
// metadata, and the verdict arrives later as a completion notification.  Two
// details are taken from a real transcript rather than guessed, because both
// defeat a naive reader: the notification is a tool_result whose text sits in
// .content rather than .text, and it is attached to whichever unrelated tool
// call happened to be in flight, so its tool_use_id does not identify the
// review.  The launch metadata deliberately carries no COMPLIANCE line, so a
// gate that read it as the report would mistake a launch for a verdict.
const TX_BACKGROUND_PASS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","id":"wait1","input":{"command":"sleep"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"wait1","content":"waited\n\n<task-notification>\n<status>completed</status>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}]}}
"#;

// The same delivery path, but the background review found something.
const TX_BACKGROUND_FINDINGS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","id":"wait1","input":{"command":"sleep"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"wait1","content":"waited\n\n<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>"}]}}
"#;

// The notification delivered as a text block rather than a tool_result, which
// is how it reads when no tool call is in flight to carry it.
const TX_BACKGROUND_PASS_TEXT: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"user","message":{"content":[{"type":"text","text":"<task-notification>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}]}}
"#;

// In practice the most common delivery: the notification text lands in a
// `toolUseResult` field beside `.message` rather than inside the content array
// at all.  Taken from a real transcript — a reader that walks the content
// blocks finds nothing here.
const TX_BACKGROUND_PASS_SIBLING: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","id":"wait1","input":{"command":"sleep"}}]}}
{"type":"user","toolUseResult":{"stdout":"waited\n\n<task-notification>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"},"message":{"content":[{"type":"tool_result","tool_use_id":"wait1","content":"waited"}]}}
"#;

// The delivery a session sees whenever no tool call is in flight when the
// review finishes: the notification is queued, and the transcript records the
// queue bookkeeping (queue-operation enqueue/remove) and the queued_command
// attachment that hands it to the model — none of them a user turn.  Taken
// from a real transcript; a reader admitting only user turns never sees the
// verdict here.
const TX_QUEUED_PASS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<status>completed</status>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}
{"type":"queue-operation","operation":"remove","content":"<task-notification>\n<status>completed</status>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}
{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<status>completed</status>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}}
"#;

// The same queued delivery carrying findings, then a later queued PASS after
// they were addressed: the most recent verdict is the one that counts, in this
// shape as in the others.
const TX_QUEUED_FINDINGS_THEN_PASS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev2","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev2","content":"Async agent launched successfully.\nagentId: def456"}]}}
{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}}
"#;

// Queued findings with nothing after them keep the gate closed.
const TX_QUEUED_FINDINGS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>"}
{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>"}}
"#;

// The assistant discussing this gate is not a verdict.  Its prose quotes
// "COMPLIANCE: PASS" whenever it explains the rule, and a notification tag
// when it explains the delivery — so reading assistant turns back would let
// the assistant release the gate by talking about it.
const TX_ASSISTANT_PROSE: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"The gate wants a <task-notification> carrying COMPLIANCE: PASS before it will release."}]}}
"#;

// This gate's own block reason, re-injected as a user turn after a passing
// review.  It must not read as a fresh prompt (which would hide the review)
// and must not be mistaken for a verdict, even though it quotes the words
// "COMPLIANCE: PASS".
const TX_HOOK_REINJECTION: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
{"type":"user","message":{"content":[{"type":"text","text":"Code or config files changed this turn, but the template-compliance review has not run.  This gate releases only when the review reports COMPLIANCE: PASS."}]}}
"#;

// A findings block quotes the report it is complaining about, so when it
// comes back as a user turn it carries both a task-notification tag and — from
// the gate's own closing sentence — the words "COMPLIANCE: PASS".  Nothing
// behind it passed, so the gate must still block: this is the shape that would
// release on the strength of the gate's own prose if block text were admitted
// as a verdict.
const TX_HOOK_REINJECTION_ONLY: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"Async agent launched successfully.\nagentId: abc123"}]}]}}
{"type":"user","message":{"content":[{"type":"text","text":"The template-compliance review reported findings that are not yet resolved:\n\n<task-notification>\n<result>check.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS</result>\n</task-notification>\n\nAddress every finding, then re-run the template-compliance subagent.  This gate releases only when the review reports COMPLIANCE: PASS."}]}}
"#;

// The clippy block reason takes the same round trip as the compliance one, and
// both filters carry a separate marker for it.  A passing review sits behind
// it, so the gate releases — unless the clippy marker is dropped, in which
// case the re-injected block reads as a fresh prompt and hides that review.
const TX_CLIPPY_REINJECTION: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
{"type":"user","message":{"content":[{"type":"text","text":"clippy reported problems on the Rust changes this turn. Resolve every warning before ending the turn."}]}}
"#;

// The assistant launched the review in the background and then waited on it
// with a blocking TaskOutput call.  The verdict comes back as TaskOutput's own
// tool_result — a string wrapping the report in <output> tags, whose
// tool_use_id matches no review call — and, because the wait consumed the
// completion, no task-notification is ever written.  Taken from a real
// transcript; a gate reading only the shapes above sees no verdict here.  The
// TaskOutput call is tied back to the review by its task_id, which is the
// agentId the launch metadata reported.
const TX_TASKOUTPUT_PASS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"Async agent launched successfully. (This tool result is internal metadata.)\nagentId: abc123 (internal ID - do not mention to user.)"}]}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskOutput","id":"out1","input":{"task_id":"abc123","block":true,"timeout":600000}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"out1","content":"<retrieval_status>success</retrieval_status>\n\n<task_id>abc123</task_id>\n\n<task_type>local_agent</task_type>\n\n<status>completed</status>\n\n<output>\nNo findings.\n\nCOMPLIANCE: PASS\n</output>"}]}}
"#;

// The same wait, but the review found something.
const TX_TASKOUTPUT_FINDINGS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"Async agent launched successfully.\nagentId: abc123"}]}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskOutput","id":"out1","input":{"task_id":"abc123","block":true,"timeout":600000}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"out1","content":"<retrieval_status>success</retrieval_status>\n\n<task_id>abc123</task_id>\n\n<task_type>local_agent</task_type>\n\n<status>completed</status>\n\n<output>\ncheck.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS\n</output>"}]}}
"#;

// A TaskOutput wait on some other background agent — not the review — whose
// output happens to quote the verdict marker.  Only the launch metadata sits
// behind the review itself, so the gate must still say the review has not
// run: the TaskOutput channel is admitted by the task_id join, not by its
// text.
const TX_TASKOUTPUT_OTHER_AGENT: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"Async agent launched successfully.\nagentId: abc123"}]}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"exp1","input":{"subagent_type":"Explore"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"exp1","content":[{"type":"text","text":"Async agent launched successfully.\nagentId: zzz999"}]}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskOutput","id":"out1","input":{"task_id":"zzz999","block":true,"timeout":600000}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"out1","content":"<retrieval_status>success</retrieval_status>\n\n<task_id>zzz999</task_id>\n\n<task_type>local_agent</task_type>\n\n<status>completed</status>\n\n<output>\nThe hook releases when it reads COMPLIANCE: PASS from the reviewer.\n</output>"}]}}
"#;

// The launch metadata text without an agentId line; the id is recorded only
// in the toolUseResult field beside the message, which the harness writes on
// the same entry.  The join must read it from there.
const TX_TASKOUTPUT_SIBLING_ID: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"abc123"},"message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"Async agent launched successfully."}]}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskOutput","id":"out1","input":{"task_id":"abc123","block":true,"timeout":600000}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"out1","content":"<retrieval_status>success</retrieval_status>\n\n<task_id>abc123</task_id>\n\n<status>completed</status>\n\n<output>\nNo findings.\n\nCOMPLIANCE: PASS\n</output>"}]}}
"#;

// Findings retrieved through TaskOutput, then a second review run in the
// foreground whose own tool_result passes.  The pass is later in the
// transcript, so it is the verdict that counts — across channels, not merely
// within one.
const TX_TASKOUTPUT_FINDINGS_THEN_PASS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":[{"type":"text","text":"Async agent launched successfully.\nagentId: abc123"}]}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskOutput","id":"out1","input":{"task_id":"abc123","block":true,"timeout":600000}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"out1","content":"<retrieval_status>success</retrieval_status>\n\n<task_id>abc123</task_id>\n\n<status>completed</status>\n\n<output>\ncheck.rs:12 uses let mut\n\nCOMPLIANCE: FINDINGS\n</output>"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev2","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev2","content":[{"type":"text","text":"COMPLIANCE: PASS"}]}]}}
"#;

// The mirror image across the two older channels: a queued notification that
// passed, then a foreground review that found something.  The findings are
// later, so they are the verdict.  A gate that gathers verdicts channel by
// channel rather than in transcript order sees the pass last and releases.
const TX_NOTIFICATION_PASS_THEN_FINDINGS: &str = r#"{"type":"user","message":{"content":"please change the code"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}
{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<result>No findings.\n\nCOMPLIANCE: PASS</result>\n</task-notification>"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev2","input":{"subagent_type":"template-compliance"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev2","content":[{"type":"text","text":"COMPLIANCE: FINDINGS\nsomething is off"}]}]}}
"#;

// ── Harness ──────────────────────────────────────────────────────────

/// One synthetic session: a scratch directory holding its fixtures and, as
/// `TMPDIR`, the gate's per-session state, so the round caps and the
/// unchanged-tree release are isolated from every other test.
struct Session {
  scratch: TempDir,
  id: &'static str,
}

/// The seam's default: a clippy command that passes without compiling.  Cases
/// that exercise the clippy gate override it with a failing command.  Without
/// this, the Rust fixtures would shell out to real clippy on every run.
const CLIPPY_PASSES: &str = "true";

impl Session {
  fn new(id: &'static str) -> Self {
    Self {
      scratch: TempDir::new().unwrap(),
      id,
    }
  }

  fn fixture(&self, name: &str, contents: &str) -> PathBuf {
    let path = self.scratch.path().join(name);
    fs::write(&path, contents).unwrap();
    path
  }

  fn transcript(&self, contents: &str) -> PathBuf {
    self.fixture("transcript.jsonl", contents)
  }

  fn files(&self, contents: &str) -> PathBuf {
    self.fixture("files", contents)
  }

  /// Invoke the gate and return its stdout.  The permission mode is sent only
  /// when a case names one; left out, the field is omitted altogether, which
  /// is what a harness that predates it sends — so every case that does not
  /// care models that harness.
  fn stop(
    &self,
    transcript: &Path,
    files: &Path,
    clippy_cmd: &str,
    permission_mode: Option<&str>,
  ) -> String {
    let stdin_json = serde_json::json!({
      "session_id": self.id,
      "transcript_path": transcript,
    });
    let stdin_json = permission_mode.map_or(stdin_json.clone(), |mode| {
      let mut with_mode = stdin_json.clone();
      with_mode["permission_mode"] =
        serde_json::Value::String(mode.to_string());
      with_mode
    });
    let mut child = Command::new(GATE)
      .env("TMPDIR", self.scratch.path())
      .env("review_stop_git_files_file", files)
      .env("review_stop_clippy_cmd", clippy_cmd)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn()
      .unwrap();
    child
      .stdin
      .take()
      .unwrap()
      .write_all(stdin_json.to_string().as_bytes())
      .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
      output.status.success(),
      "the gate must exit zero; stderr: {}",
      String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
  }
}

fn assert_releases(stdout: &str) {
  assert!(
    !stdout.contains("\"decision\""),
    "expected release, got block: {stdout}"
  );
}

fn assert_blocks(stdout: &str, reason_contains: &str) {
  assert!(
    stdout.contains("\"block\""),
    "expected block, got: {}",
    if stdout.is_empty() { "<empty>" } else { stdout }
  );
  assert!(
    stdout.contains(reason_contains),
    "block reason missing '{reason_contains}': {stdout}"
  );
}

// ── Cases ────────────────────────────────────────────────────────────

/// Prose-only changes are gated like any other: documentation carries
/// conventions of its own, so a turn that edited only prose still needs the
/// review.
#[test]
fn prose_only_changes_block() {
  let s = Session::new("prose");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_NOREVIEW),
      &s.files(FILES_PROSE),
      CLIPPY_PASSES,
      None,
    ),
    "has not run",
  );
}

/// No working-tree changes at all: nothing to gate.
#[test]
fn no_changes_releases() {
  let s = Session::new("none");
  assert_releases(&s.stop(
    &s.transcript(TX_NOREVIEW),
    &s.files(FILES_NONE),
    CLIPPY_PASSES,
    None,
  ));
}

/// Code changed and no review ran: block with the "has not run" reason.
#[test]
fn code_without_review_blocks() {
  let s = Session::new("code-noreview");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_NOREVIEW),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      None,
    ),
    "has not run",
  );
}

/// A passing review recorded under the "Agent" tool name releases the gate.
#[test]
fn agent_review_pass_releases() {
  let s = Session::new("agent-pass");
  assert_releases(&s.stop(
    &s.transcript(TX_AGENT_PASS),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// A passing review recorded under the "Task" tool name releases the gate.
#[test]
fn task_review_pass_releases() {
  let s = Session::new("task-pass");
  assert_releases(&s.stop(
    &s.transcript(TX_TASK_PASS),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// A background review whose verdict arrives as a task-notification, rather
/// than in the tool_result, still releases the gate.  Without this the launch
/// metadata is the only thing the gate can see, and it never converges.
#[test]
fn background_review_pass_releases() {
  let s = Session::new("background-pass");
  assert_releases(&s.stop(
    &s.transcript(TX_BACKGROUND_PASS),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// The same delivery path must carry findings through as well.
#[test]
fn background_review_findings_blocks() {
  let s = Session::new("background-findings");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_BACKGROUND_FINDINGS),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      None,
    ),
    "reported findings",
  );
}

/// The notification also releases the gate when it arrives as a text block.
#[test]
fn background_review_text_notification_releases() {
  let s = Session::new("background-pass-text");
  assert_releases(&s.stop(
    &s.transcript(TX_BACKGROUND_PASS_TEXT),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// The notification also releases the gate when it arrives in a
/// toolUseResult field beside the message.
#[test]
fn background_review_sibling_notification_releases() {
  let s = Session::new("background-pass-sibling");
  assert_releases(&s.stop(
    &s.transcript(TX_BACKGROUND_PASS_SIBLING),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// A verdict recorded only as queue bookkeeping and a queued_command
/// attachment — no user turn carries it — still releases the gate.
#[test]
fn queued_notification_releases() {
  let s = Session::new("queued-pass");
  assert_releases(&s.stop(
    &s.transcript(TX_QUEUED_PASS),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// Queued findings followed by a queued pass release: the latest verdict
/// wins.
#[test]
fn queued_findings_then_pass_releases() {
  let s = Session::new("queued-findings-then-pass");
  assert_releases(&s.stop(
    &s.transcript(TX_QUEUED_FINDINGS_THEN_PASS),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// Queued findings with no later pass keep the gate closed.
#[test]
fn queued_findings_blocks() {
  let s = Session::new("queued-findings");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_QUEUED_FINDINGS),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      None,
    ),
    "reported findings",
  );
}

/// The assistant talking about the gate never satisfies it.
#[test]
fn assistant_prose_is_not_a_verdict() {
  let s = Session::new("assistant-prose");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_ASSISTANT_PROSE),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      None,
    ),
    "has not run",
  );
}

/// The gate's own block reason, re-injected as a user turn, is not a new
/// prompt: the passing review before it still counts, so the gate releases
/// instead of spinning.
#[test]
fn hook_reinjection_keeps_the_turn() {
  let s = Session::new("reinjection");
  assert_releases(&s.stop(
    &s.transcript(TX_HOOK_REINJECTION),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// A re-injected findings block is never itself a verdict, even though it
/// carries a task-notification tag and quotes "COMPLIANCE: PASS" while
/// explaining the rule.  Only the launch metadata sits behind it, so the gate
/// stays closed.
#[test]
fn hook_reinjection_is_not_a_verdict() {
  let s = Session::new("reinjection-only");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_HOOK_REINJECTION_ONLY),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      None,
    ),
    "has not run",
  );
}

/// The clippy block reason gets the same treatment as the compliance one: it
/// is not a fresh prompt, so the passing review before it still counts.
#[test]
fn clippy_reinjection_keeps_the_turn() {
  let s = Session::new("clippy-reinjection");
  assert_releases(&s.stop(
    &s.transcript(TX_CLIPPY_REINJECTION),
    &s.files(FILES_CODE_NONRUST),
    CLIPPY_PASSES,
    None,
  ));
}

/// A review reporting findings keeps the gate closed.
#[test]
fn findings_blocks() {
  let s = Session::new("findings");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_FINDINGS),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      None,
    ),
    "reported findings",
  );
}

/// A passing review from before the latest prompt does not satisfy this turn.
#[test]
fn stale_review_blocks() {
  let s = Session::new("stale");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_STALE_REVIEW),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      None,
    ),
    "has not run",
  );
}

/// A missing transcript leaves nothing to gate on.
#[test]
fn missing_transcript_releases() {
  let s = Session::new("missing");
  assert_releases(&s.stop(
    &s.scratch.path().join("does-not-exist.jsonl"),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// Plan mode is read-only, so the gate stands aside: code changes sit in the
/// tree and no review has run, yet the turn ends.  This is also the release
/// valve for stepping out of a gated task into a discussion.
#[test]
fn plan_mode_releases() {
  let s = Session::new("plan");
  assert_releases(&s.stop(
    &s.transcript(TX_NOREVIEW),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    Some("plan"),
  ));
}

/// Only plan mode is read-only.  Any other named mode is gated as before, so
/// a release here would mean the gate keys on the field being present rather
/// than on its value.
#[test]
fn non_plan_mode_still_blocks() {
  let s = Session::new("not-plan");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_NOREVIEW),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      Some("default"),
    ),
    "has not run",
  );
}

/// A passing review releases and records what it released on.  A later Stop
/// in the same session — a new prompt with no review since — that finds the
/// same working tree has nothing new to review, so it releases rather than
/// demanding the review again.  Without this every discussion turn after
/// reviewed but uncommitted edits re-blocks.  Once the tree has moved on,
/// though, the recorded fingerprint no longer matches, so the gate re-arms
/// and demands a review.  (One test, because the three Stops share a
/// session's state.)
#[test]
fn unchanged_tree_releases_after_pass_and_a_changed_tree_rearms() {
  let s = Session::new("unchanged");
  assert_releases(&s.stop(
    &s.transcript(TX_AGENT_PASS),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
  assert_releases(&s.stop(
    &s.transcript(TX_STALE_REVIEW),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
  assert_blocks(
    &s.stop(
      &s.transcript(TX_STALE_REVIEW),
      &s.files(FILES_CODE_TWO),
      CLIPPY_PASSES,
      None,
    ),
    "has not run",
  );
}

/// MAX_ROUNDS safety valve: after the bounded number of consecutive blocks
/// within one turn, the gate releases so a finding the assistant genuinely
/// cannot resolve does not wedge the session: the invocation after the cap
/// releases.  Giving up also records the tree, so the next Stop on the same
/// content releases at once rather than opening another cycle.
#[test]
fn max_rounds_releases() {
  let s = Session::new("rounds");
  let transcript = s.transcript(TX_NOREVIEW);
  let files = s.files(FILES_CODE);
  for _ in 0..4 {
    assert_blocks(
      &s.stop(&transcript, &files, CLIPPY_PASSES, None),
      "has not run",
    );
  }
  assert_releases(&s.stop(&transcript, &files, CLIPPY_PASSES, None));
  assert_releases(&s.stop(&transcript, &files, CLIPPY_PASSES, None));
}

/// A Rust change with a failing clippy gate blocks before the review is even
/// consulted — the transcript here carries a passing review, so a release
/// would prove clippy did not gate.
#[test]
fn clippy_failure_blocks() {
  let s = Session::new("clippy-fail");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_AGENT_PASS),
      &s.files(FILES_CODE),
      "echo clippy-boom; exit 1",
      None,
    ),
    "clippy reported problems",
  );
}

/// The clippy gate only fires for Rust files.  A non-Rust code change with a
/// failing clippy command must skip clippy and fall through to the review
/// gate, blocking with the review reason rather than the clippy one.
#[test]
fn clippy_skipped_for_non_rust() {
  let s = Session::new("clippy-skip");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_NOREVIEW),
      &s.files(FILES_CODE_NONRUST),
      "echo clippy-boom; exit 1",
      None,
    ),
    "has not run",
  );
}

/// clippy safety valve: after MAX_ROUNDS consecutive failing clippy runs the
/// gate releases (then falls through to the review, which passes here), so an
/// unfixable warning cannot wedge the session.
#[test]
fn clippy_max_rounds_releases() {
  let s = Session::new("clippy-rounds");
  let transcript = s.transcript(TX_AGENT_PASS);
  let files = s.files(FILES_CODE);
  for _ in 0..4 {
    assert_blocks(
      &s.stop(&transcript, &files, "exit 1", None),
      "clippy reported problems",
    );
  }
  assert_releases(&s.stop(&transcript, &files, "exit 1", None));
}

/// A background review the assistant waited on with a blocking TaskOutput
/// call: the verdict arrives as TaskOutput's own tool_result, and no
/// notification is ever written because the wait consumed the completion.
/// The gate must read it, or a real pass looks like a review that never ran.
#[test]
fn taskoutput_review_pass_releases() {
  let s = Session::new("taskoutput-pass");
  assert_releases(&s.stop(
    &s.transcript(TX_TASKOUTPUT_PASS),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// The same channel carries findings through, and the block names them.
#[test]
fn taskoutput_review_findings_blocks() {
  let s = Session::new("taskoutput-findings");
  let stdout = s.stop(
    &s.transcript(TX_TASKOUTPUT_FINDINGS),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  );
  assert_blocks(&stdout, "reported findings");
  assert_blocks(&stdout, "uses let mut");
}

/// A TaskOutput result is a verdict only when it waited on the review agent.
/// Any other agent's output — even one that quotes "COMPLIANCE: PASS" —
/// leaves the gate exactly where it was.
#[test]
fn taskoutput_other_agent_is_not_a_verdict() {
  let s = Session::new("taskoutput-other");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_TASKOUTPUT_OTHER_AGENT),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      None,
    ),
    "has not run",
  );
}

/// The agent id may be recorded only in the launch entry's toolUseResult
/// sibling rather than in the result text; the join must find it there too.
#[test]
fn taskoutput_sibling_agent_id_releases() {
  let s = Session::new("taskoutput-sibling");
  assert_releases(&s.stop(
    &s.transcript(TX_TASKOUTPUT_SIBLING_ID),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// Findings retrieved through TaskOutput, then a foreground re-run that
/// passes: the later verdict wins even though the two arrived on different
/// channels.
#[test]
fn taskoutput_findings_then_pass_releases() {
  let s = Session::new("taskoutput-then-pass");
  assert_releases(&s.stop(
    &s.transcript(TX_TASKOUTPUT_FINDINGS_THEN_PASS),
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    None,
  ));
}

/// The mirror image: a notification pass followed by a foreground review that
/// found something must block.  A gate that ranks verdicts by channel rather
/// than by position releases here on the stale pass.
#[test]
fn notification_pass_then_findings_blocks() {
  let s = Session::new("notification-then-findings");
  assert_blocks(
    &s.stop(
      &s.transcript(TX_NOTIFICATION_PASS_THEN_FINDINGS),
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      None,
    ),
    "reported findings",
  );
}
