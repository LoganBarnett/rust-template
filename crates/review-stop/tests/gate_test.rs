//! Black-box tests for the code-review Stop hook's gate.
//!
//! The gate lists the working tree's changes, runs a clippy gate when Rust
//! files changed, then runs the reviewer and either releases the turn or emits
//! a block decision.  Every input is injectable through a seam, so the cases
//! drive every branch without a real compile or a real reviewer; the reviewer
//! seam prints the JSON envelope `claude --print --output-format json` would.
//! The verdict cache lives under `TMPDIR`, pointed at a scratch directory per
//! case, so cases that span several Stops share state only with themselves.
//!
//! The binary is invoked from this crate's directory, inside the repository,
//! so its git probes succeed the way they do in a real session; the cases
//! about what the packet carries build their own repositories instead.

// clippy's in-test heuristic does not cover free helper fns in an integration
// test binary, so the exemption is stated at file level.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

/// The gate binary under test, resolved by cargo for this crate's tests.
const GATE: &str = env!("CARGO_BIN_EXE_rust-template-review-stop");

// ── Fixtures ─────────────────────────────────────────────────────────

// Working-tree file lists.  Every changed file is gated, prose included; Rust
// files also arm the clippy gate.
const FILES_PROSE: &str = "README.org\ndocs/notes.txt\n";
const FILES_CODE: &str = "src/main.rs\n";
// The same change plus one more file: a different tree from FILES_CODE, so a
// verdict cached for that one does not apply.
const FILES_CODE_TWO: &str = "src/main.rs\nsrc/lib.rs\n";
// A change that is not Rust: gated for review, but the clippy gate must leave
// it alone.
const FILES_CODE_NONRUST: &str = "flake.nix\n";
const FILES_NONE: &str = "";

/// The clippy seam's default: passes without compiling.
const CLIPPY_PASSES: &str = "true";
const CLIPPY_FAILS: &str = "echo 'warning: unused variable `x`' >&2; false";

/// The envelope a clean review produces.
const VERDICT_PASS: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"{\"findings\":[]}","structured_output":{"findings":[]}}"#;
/// One finding, with every field the block reason renders.
const VERDICT_FINDINGS: &str = r#"{"type":"result","subtype":"success","is_error":false,"structured_output":{"findings":[{"path":"src/main.rs","line":3,"convention":"comments are complete sentences","document":"llms.org","fix":"end the comment with a period"}]}}"#;
/// An envelope that names an error rather than a verdict.
const VERDICT_ERROR: &str = r#"{"type":"result","subtype":"error","is_error":true,"result":"rate limited"}"#;

// ── Harness ──────────────────────────────────────────────────────────

/// One synthetic session: a scratch directory holding its fixtures and, as
/// `TMPDIR`, the gate's verdict cache.  `HOME` points there too, so the
/// packet never picks up the developer's real global instructions.
struct Session {
  scratch: TempDir,
}

impl Session {
  fn new() -> Self {
    Self {
      scratch: TempDir::new().unwrap(),
    }
  }

  fn fixture(&self, name: &str, contents: &str) -> PathBuf {
    let path = self.scratch.path().join(name);
    fs::write(&path, contents).unwrap();
    path
  }

  fn files(&self, contents: &str) -> PathBuf {
    self.fixture("files", contents)
  }

  fn marker(&self) -> PathBuf {
    self.scratch.path().join("invocations")
  }

  /// A reviewer seam that records each invocation, then prints `envelope`.
  /// The seams drain stdin (`cat >/dev/null`) so they model a reviewer that
  /// actually reads the packet; one that exits without reading is rejected, and
  /// `reviewer_that_ignores_the_packet_blocks` exercises that path on purpose.
  fn reviewer(&self, envelope: &str) -> String {
    format!(
      "echo run >> '{}'; cat >/dev/null; printf '%s' '{envelope}'",
      self.marker().display()
    )
  }

  /// A reviewer seam that records the invocation and fails without a verdict.
  fn failing_reviewer(&self) -> String {
    format!(
      "echo run >> '{}'; cat >/dev/null; echo 'reviewer crashed' >&2; exit 3",
      self.marker().display()
    )
  }

  /// A reviewer seam that saves the packet it was fed, then passes.
  fn recording_reviewer(&self) -> (String, PathBuf) {
    let dump = self.scratch.path().join("packet");
    (
      format!(
        "echo run >> '{}'; cat > '{}'; printf '%s' '{VERDICT_PASS}'",
        self.marker().display(),
        dump.display()
      ),
      dump,
    )
  }

  fn invocations(&self) -> usize {
    fs::read_to_string(self.marker())
      .map(|text| text.lines().count())
      .unwrap_or(0)
  }

  fn gate(&self) -> Command {
    let mut command = Command::new(GATE);
    command
      .env("TMPDIR", self.scratch.path())
      .env("HOME", self.scratch.path());
    command
  }

  /// Invoke the gate as the Stop hook and return its stdout.  A missing file
  /// list means the gate reads git for real, which the packet cases rely on.
  fn stop_with(&self, stop: &Stop) -> String {
    let stdin_json = stop.permission_mode.map_or_else(
      || serde_json::json!({}),
      |mode| serde_json::json!({ "permission_mode": mode }),
    );
    let mut command = self.gate();
    if let Some(files) = stop.files {
      command.env("review_stop_git_files_file", files);
    }
    if let Some(dir) = stop.cwd {
      command.current_dir(dir);
    }
    command
      .env("review_stop_clippy_cmd", stop.clippy_cmd)
      .env("review_stop_reviewer_cmd", stop.reviewer_cmd)
      .envs(stop.extra_env.iter().cloned());
    let output = run(command, Some(&stdin_json.to_string()));
    assert!(
      output.status.success(),
      "the gate must exit zero; stderr: {}",
      String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
  }

  fn stop(&self, files: &Path, clippy_cmd: &str, reviewer_cmd: &str) -> String {
    self.stop_with(&Stop::new(files, clippy_cmd, reviewer_cmd))
  }
}

/// One Stop invocation's inputs.
struct Stop<'a> {
  files: Option<&'a Path>,
  clippy_cmd: &'a str,
  reviewer_cmd: &'a str,
  permission_mode: Option<&'a str>,
  extra_env: Vec<(String, OsString)>,
  cwd: Option<&'a Path>,
}

impl<'a> Stop<'a> {
  fn new(files: &'a Path, clippy_cmd: &'a str, reviewer_cmd: &'a str) -> Self {
    Self {
      files: Some(files),
      clippy_cmd,
      reviewer_cmd,
      permission_mode: None,
      extra_env: Vec::new(),
      cwd: None,
    }
  }

  /// A Stop that lists changes from git itself, inside `repo`.
  fn in_repo(repo: &'a Path, reviewer_cmd: &'a str) -> Self {
    Self {
      files: None,
      clippy_cmd: CLIPPY_PASSES,
      reviewer_cmd,
      permission_mode: None,
      extra_env: Vec::new(),
      cwd: Some(repo),
    }
  }
}

fn run(mut command: Command, stdin: Option<&str>) -> Output {
  let mut child = command
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
  if let Some(text) = stdin {
    child
      .stdin
      .take()
      .unwrap()
      .write_all(text.as_bytes())
      .unwrap();
  }
  // Whether or not anything was written, the pipe must close so a gate that
  // reads stdin sees end-of-input.
  drop(child.stdin.take());
  child.wait_with_output().unwrap()
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

/// A git repository under the session's scratch directory with one committed
/// convention document, so a case can put a different rule in the working
/// copy and see which one the packet carries.
fn repo_with_committed_rule(session: &Session, rule: &str) -> PathBuf {
  let repo = session.scratch.path().join("repo");
  fs::create_dir(&repo).unwrap();
  git(&repo, &["init", "--quiet"]);
  fs::write(repo.join("CONTRIBUTING.org"), format!("* Rules\n\n{rule}\n"))
    .unwrap();
  git(&repo, &["add", "CONTRIBUTING.org"]);
  git(&repo, &["commit", "--quiet", "--message", "rules"]);
  repo
}

/// Run git in `repo` with identity, signing, and hooks pinned so the
/// developer's global configuration cannot interfere.  (`git -c` sets a config
/// value for one invocation; git has no long-form spelling of the flag.)
fn git(repo: &Path, args: &[&str]) {
  let status = Command::new("git")
    .current_dir(repo)
    .args([
      "-c",
      "user.name=gate-test",
      "-c",
      "user.email=gate-test@example.invalid",
      "-c",
      "commit.gpgsign=false",
      "-c",
      "core.hooksPath=/dev/null",
    ])
    .args(args)
    .status()
    .unwrap();
  assert!(status.success(), "git {args:?} failed");
}

/// The section of `packet` between the BEGIN and END markers for `label`.
fn section<'a>(packet: &'a str, label: &str) -> &'a str {
  let begin = format!("----- BEGIN {label} -----\n");
  let end = format!("\n----- END {label} -----");
  let start = packet
    .find(&begin)
    .unwrap_or_else(|| panic!("no {label} section in:\n{packet}"))
    + begin.len();
  let stop = packet[start..].find(&end).unwrap() + start;
  &packet[start..stop]
}

/// The current PATH with every directory that offers a `claude` removed.
fn path_without_claude() -> OsString {
  std::env::join_paths(
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
      .filter(|dir| !dir.join("claude").is_file()),
  )
  .unwrap()
}

// ── Release valves ───────────────────────────────────────────────────

/// No working-tree changes at all: nothing to gate, nothing to review.
#[test]
fn no_changes_releases_without_review() {
  let s = Session::new();
  assert_releases(&s.stop(
    &s.files(FILES_NONE),
    CLIPPY_PASSES,
    &s.failing_reviewer(),
  ));
  assert_eq!(s.invocations(), 0);
}

/// Plan mode is read-only, so the gate stands aside before reviewing.
#[test]
fn plan_mode_releases_without_review() {
  let s = Session::new();
  let files = s.files(FILES_CODE);
  let reviewer = s.failing_reviewer();
  let stop = Stop {
    permission_mode: Some("plan"),
    ..Stop::new(&files, CLIPPY_PASSES, &reviewer)
  };
  assert_releases(&s.stop_with(&stop));
  assert_eq!(s.invocations(), 0);
}

/// The nested reviewer's own Stop hook must not review the reviewer.
#[test]
fn nested_reviewer_run_releases_without_review() {
  let s = Session::new();
  let files = s.files(FILES_CODE);
  let reviewer = s.failing_reviewer();
  let stop = Stop {
    extra_env: vec![(
      "RUST_TEMPLATE_REVIEW_NESTED".to_string(),
      OsString::from("1"),
    )],
    ..Stop::new(&files, CLIPPY_PASSES, &reviewer)
  };
  assert_releases(&s.stop_with(&stop));
  assert_eq!(s.invocations(), 0);
}

// ── Clippy ───────────────────────────────────────────────────────────

/// A clippy failure blocks before the reviewer is ever consulted.
#[test]
fn clippy_failure_blocks_before_review() {
  let s = Session::new();
  assert_blocks(
    &s.stop(&s.files(FILES_CODE), CLIPPY_FAILS, &s.failing_reviewer()),
    "clippy reported problems",
  );
  assert_eq!(s.invocations(), 0);
}

/// Changes with no Rust in them never arm the clippy gate.
#[test]
fn non_rust_changes_skip_clippy() {
  let s = Session::new();
  assert_releases(&s.stop(
    &s.files(FILES_CODE_NONRUST),
    CLIPPY_FAILS,
    &s.reviewer(VERDICT_PASS),
  ));
  assert_eq!(s.invocations(), 1);
}

// ── Verdicts ─────────────────────────────────────────────────────────

/// Prose-only changes are reviewed like any other: documentation carries
/// conventions of its own.
#[test]
fn prose_only_changes_are_reviewed() {
  let s = Session::new();
  assert_releases(&s.stop(
    &s.files(FILES_PROSE),
    CLIPPY_PASSES,
    &s.reviewer(VERDICT_PASS),
  ));
  assert_eq!(s.invocations(), 1);
}

#[test]
fn clean_review_releases() {
  let s = Session::new();
  assert_releases(&s.stop(
    &s.files(FILES_CODE),
    CLIPPY_PASSES,
    &s.reviewer(VERDICT_PASS),
  ));
  assert_eq!(s.invocations(), 1);
}

/// Findings block, and the reason carries everything the agent needs to act
/// on each one.
#[test]
fn findings_block_with_their_details() {
  let s = Session::new();
  let stdout =
    s.stop(&s.files(FILES_CODE), CLIPPY_PASSES, &s.reviewer(VERDICT_FINDINGS));
  assert_blocks(&stdout, "src/main.rs:3");
  for detail in [
    "comments are complete sentences",
    "llms.org",
    "end the comment with a period",
    "Address every finding",
  ] {
    assert!(stdout.contains(detail), "reason missing '{detail}': {stdout}");
  }
}

// ── Cache ────────────────────────────────────────────────────────────

/// An unchanged tree that was reviewed clean ends later turns without
/// another review.
#[test]
fn clean_verdict_is_cached_for_the_same_tree() {
  let s = Session::new();
  let files = s.files(FILES_CODE);
  assert_releases(&s.stop(&files, CLIPPY_PASSES, &s.reviewer(VERDICT_PASS)));
  assert_releases(&s.stop(&files, CLIPPY_PASSES, &s.failing_reviewer()));
  assert_eq!(s.invocations(), 1);
}

/// Findings stay in force, without another review, until the tree changes.
#[test]
fn findings_are_cached_until_the_tree_changes() {
  let s = Session::new();
  let files = s.files(FILES_CODE);
  assert_blocks(
    &s.stop(&files, CLIPPY_PASSES, &s.reviewer(VERDICT_FINDINGS)),
    "src/main.rs:3",
  );
  assert_blocks(
    &s.stop(&files, CLIPPY_PASSES, &s.failing_reviewer()),
    "src/main.rs:3",
  );
  assert_eq!(s.invocations(), 1);
}

/// A different tree is a different review.
#[test]
fn changed_tree_is_reviewed_again() {
  let s = Session::new();
  let reviewer = s.reviewer(VERDICT_PASS);
  assert_releases(&s.stop(&s.files(FILES_CODE), CLIPPY_PASSES, &reviewer));
  assert_releases(&s.stop(&s.files(FILES_CODE_TWO), CLIPPY_PASSES, &reviewer));
  assert_eq!(s.invocations(), 2);
}

// ── Failures block ───────────────────────────────────────────────────

/// A reviewer that fails to run is not a pass; the gate blocks with the
/// failure so the turn cannot end unreviewed.
#[test]
fn reviewer_failure_blocks() {
  let s = Session::new();
  let stdout =
    s.stop(&s.files(FILES_CODE), CLIPPY_PASSES, &s.failing_reviewer());
  assert_blocks(&stdout, "reviewer exited with");
  assert!(stdout.contains("reviewer crashed"), "{stdout}");
}

#[test]
fn reviewer_error_envelope_blocks() {
  let s = Session::new();
  assert_blocks(
    &s.stop(&s.files(FILES_CODE), CLIPPY_PASSES, &s.reviewer(VERDICT_ERROR)),
    "rate limited",
  );
}

#[test]
fn unparseable_verdict_blocks() {
  let s = Session::new();
  assert_blocks(
    &s.stop(
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      "cat >/dev/null; printf 'not json'",
    ),
    "could not read a verdict",
  );
}

/// A reviewer that exits successfully without reading the packet never saw
/// what it was meant to judge, so its verdict must not be trusted — the gate
/// blocks rather than accept a pass from a reviewer that ignored its input.
#[test]
fn reviewer_that_ignores_the_packet_blocks() {
  let s = Session::new();
  assert_blocks(
    &s.stop(
      &s.files(FILES_CODE),
      CLIPPY_PASSES,
      &format!("printf '%s' '{VERDICT_PASS}'"),
    ),
    "without reading the review packet",
  );
}

/// A reviewer that runs past the deadline is killed and blocks, so a slow
/// review fails closed here instead of being cancelled by the hook timeout and
/// letting the turn end unreviewed.
#[test]
fn slow_reviewer_times_out_and_blocks() {
  let s = Session::new();
  let files = s.files(FILES_CODE);
  let seam = format!("cat >/dev/null; sleep 5; printf '%s' '{VERDICT_PASS}'");
  let stop = Stop {
    extra_env: vec![(
      "review_stop_reviewer_timeout_secs".to_string(),
      OsString::from("1"),
    )],
    ..Stop::new(&files, CLIPPY_PASSES, &seam)
  };
  assert_blocks(&s.stop_with(&stop), "did not finish within 1 seconds");
}

/// Without the seam the gate wants a real `claude`; a PATH without one is a
/// block, never a release.
#[test]
fn missing_reviewer_blocks() {
  let s = Session::new();
  let files = s.files(FILES_CODE);
  let stop = Stop {
    extra_env: vec![("PATH".to_string(), path_without_claude())],
    ..Stop::new(&files, CLIPPY_PASSES, "")
  };
  assert_blocks(&s.stop_with(&stop), "not on PATH");
}

// ── The packet ───────────────────────────────────────────────────────

/// The conventions come from HEAD, so an uncommitted change to a rule reaches
/// the reviewer only as a change under review, never as the rule itself.
/// Untracked files travel whole, except a top-level licence.
#[test]
fn packet_carries_committed_conventions_and_untracked_files() {
  let s = Session::new();
  let repo = repo_with_committed_rule(&s, "RULE ALPHA applies.");
  fs::write(repo.join("CONTRIBUTING.org"), "* Rules\n\nRULE BETA applies.\n")
    .unwrap();
  fs::write(repo.join("notes.txt"), "UNTRACKED-MARKER\n").unwrap();
  fs::write(repo.join("LICENSE"), "MIT\n").unwrap();
  let (reviewer, dump) = s.recording_reviewer();
  assert_releases(&s.stop_with(&Stop::in_repo(&repo, &reviewer)));
  let packet = fs::read_to_string(dump).unwrap();
  let conventions = section(&packet, "CONVENTIONS: CONTRIBUTING.org");
  assert!(conventions.contains("RULE ALPHA"), "{packet}");
  assert!(!conventions.contains("RULE BETA"), "{packet}");
  assert!(packet.contains("+RULE BETA applies."), "{packet}");
  assert!(
    section(&packet, "UNTRACKED FILE: notes.txt").contains("UNTRACKED-MARKER"),
    "{packet}"
  );
  assert!(!packet.contains("UNTRACKED FILE: LICENSE"), "{packet}");
}

/// With no commit yet there is no committed rule to defend, so the working
/// copies stand in and every file shows as added against the empty tree.
#[test]
fn unborn_repository_reviews_against_the_working_copies() {
  let s = Session::new();
  let repo = s.scratch.path().join("unborn");
  fs::create_dir(&repo).unwrap();
  git(&repo, &["init", "--quiet"]);
  fs::write(repo.join("CONTRIBUTING.org"), "* Rules\n\nRULE GAMMA applies.\n")
    .unwrap();
  git(&repo, &["add", "CONTRIBUTING.org"]);
  let (reviewer, dump) = s.recording_reviewer();
  assert_releases(&s.stop_with(&Stop::in_repo(&repo, &reviewer)));
  let packet = fs::read_to_string(dump).unwrap();
  assert!(
    section(&packet, "CONVENTIONS: CONTRIBUTING.org").contains("RULE GAMMA"),
    "{packet}"
  );
  assert!(packet.contains("+RULE GAMMA applies."), "{packet}");
}

// ── Manual review ────────────────────────────────────────────────────

/// `--review` runs the same review for a human, prints the findings, and
/// carries the verdict in the exit code.
#[test]
fn review_flag_reports_findings_and_fails() {
  let s = Session::new();
  let mut command = s.gate();
  command
    .args(["--review", "true"])
    .env("review_stop_git_files_file", s.files(FILES_CODE))
    .env("review_stop_reviewer_cmd", s.reviewer(VERDICT_FINDINGS));
  let output = run(command, None);
  assert!(!output.status.success());
  let stdout = String::from_utf8(output.stdout).unwrap();
  assert!(stdout.contains("src/main.rs:3"), "{stdout}");
  assert!(stdout.contains("end the comment with a period"), "{stdout}");
}

#[test]
fn review_flag_reports_a_clean_tree() {
  let s = Session::new();
  let mut command = s.gate();
  command
    .args(["--review", "true"])
    .env("review_stop_git_files_file", s.files(FILES_CODE))
    .env("review_stop_reviewer_cmd", s.reviewer(VERDICT_PASS));
  let output = run(command, None);
  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "No findings.\n");
}
