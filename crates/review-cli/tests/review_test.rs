//! Black-box cases over the real binary.
//!
//! Each case gets its own git repository, because the review record lives at
//! the repository root: a case run against the checkout under test would write
//! into it and share state with every other case.  The reviewer is a shell
//! stand-in that records what it was handed, so a case can assert on the packet
//! rather than only on what was printed.

// Clippy's in-test exemption does not reach the free helper functions of an
// integration-test binary, so the panicking variants are permitted at file
// level here — panicking is the failure signal a test wants.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

/// The result envelope `claude --print --output-format json` prints, which the
/// reviewer stand-in has to imitate.
fn envelope(findings: &[String]) -> String {
  format!(
    r#"{{"is_error":false,"structured_output":{{"findings":[{}]}}}}"#,
    findings.join(","),
  )
}

/// A finding as the reviewer's structured output spells it.
fn finding(path: &str, convention: &str) -> String {
  format!(
    r#"{{"path":"{path}","line":1,"convention":"{convention}","document":"llms.org","fix":"do the thing"}}"#
  )
}

struct Repo {
  /// Holds both the repository and the case's scratch files; dropping it
  /// removes them.
  root: TempDir,
}

impl Repo {
  fn new() -> Self {
    let this = Self::init("main");
    // Mirrors the line this repository's own ignore file carries; a case that
    // is specifically about the in-binary filter removes it.
    this.write(".gitignore", "review.json*\n");
    this.write("README.org", "#+title: Case\n");
    this.commit("initial");
    this
  }

  /// A repository with no commit yet, so there is no `HEAD` to diff against.
  fn unborn() -> Self {
    Self::init("main")
  }

  /// An initialised repository whose first commit will land on `branch`.
  fn init(branch: &str) -> Self {
    let root = TempDir::new().expect("a scratch directory");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("the repository directory");
    std::fs::create_dir_all(root.path().join("work")).expect("the work dir");
    let this = Self { root };
    this.git(&["init", &format!("--initial-branch={branch}")]);
    this.identify();
    this
  }

  /// A clone of this repository, which is how a checkout comes to carry
  /// `origin/HEAD`.
  fn clone_of(&self) -> Self {
    let root = TempDir::new().expect("a scratch directory");
    std::fs::create_dir_all(root.path().join("work")).expect("the work dir");
    let this = Self { root };
    let output = Command::new("git")
      .arg("clone")
      .arg("--quiet")
      .arg(self.dir())
      .arg(this.dir())
      .output()
      .expect("git to run");
    assert!(
      output.status.success(),
      "git clone failed: {}",
      String::from_utf8_lossy(&output.stderr),
    );
    this.identify();
    this
  }

  /// Pin the identity and signing so the developer's own configuration
  /// cannot interfere with a commit.
  fn identify(&self) {
    self.git(&["config", "user.email", "case@example.invalid"]);
    self.git(&["config", "user.name", "Case"]);
    self.git(&["config", "commit.gpgsign", "false"]);
  }

  /// A `PATH` holding only what the reviewer stand-in needs — `bash` and
  /// `cat` — so a review that reached for `git` would not find it.
  #[cfg(unix)]
  fn path_without_git(&self) -> String {
    let bin = self.work().join("bin");
    std::fs::create_dir_all(&bin).expect("the bin dir");
    let on_path = std::env::var_os("PATH").expect("a PATH to search");
    ["bash", "cat"].iter().for_each(|tool| {
      let found = std::env::split_paths(&on_path)
        .map(|dir| dir.join(tool))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("{tool} on PATH"));
      std::os::unix::fs::symlink(found, bin.join(tool))
        .expect("to link the tool");
    });
    bin.to_string_lossy().into_owned()
  }

  fn dir(&self) -> PathBuf {
    self.root.path().join("repo")
  }

  fn work(&self) -> PathBuf {
    self.root.path().join("work")
  }

  fn git(&self, args: &[&str]) {
    let output = Command::new("git")
      .current_dir(self.dir())
      .args(args)
      .output()
      .expect("git to run");
    assert!(
      output.status.success(),
      "git {args:?} failed: {}",
      String::from_utf8_lossy(&output.stderr),
    );
  }

  fn write(&self, path: &str, contents: &str) {
    let full = self.dir().join(path);
    full
      .parent()
      .map(std::fs::create_dir_all)
      .transpose()
      .expect("the parent directory");
    std::fs::write(full, contents).expect("to write the file");
  }

  fn commit(&self, message: &str) {
    self.git(&["add", "--all"]);
    self.git(&["commit", "--message", message]);
  }

  /// A reviewer stand-in that reports `findings` and leaves a trace of what it
  /// was handed.
  fn reviewer(&self, findings: &[String]) -> String {
    let envelope = envelope(findings);
    format!(
      "cat > {packet}; echo ran >> {calls}; printf '%s' '{envelope}'",
      packet = self.work().join("packet.txt").display(),
      calls = self.work().join("calls").display(),
    )
  }

  /// The packet the reviewer was last handed.
  fn packet(&self) -> String {
    std::fs::read_to_string(self.work().join("packet.txt")).unwrap_or_default()
  }

  /// How many times the reviewer has been invoked across this case.
  fn calls(&self) -> usize {
    std::fs::read_to_string(self.work().join("calls"))
      .map(|text| text.lines().count())
      .unwrap_or(0)
  }

  fn record(&self) -> serde_json::Value {
    serde_json::from_str(
      &std::fs::read_to_string(self.dir().join("review.json"))
        .expect("a review record"),
    )
    .expect("the record to be JSON")
  }

  fn record_exists(&self) -> bool {
    self.dir().join("review.json").exists()
  }

  /// Run the binary in this repository with the given reviewer stand-in.
  fn review(&self, reviewer: &str, args: &[&str]) -> Output {
    self.review_env(reviewer, args, &[])
  }

  /// As `review`, with extra environment — colour forcing, chiefly, which a
  /// captured run would otherwise never see.
  fn review_env(
    &self,
    reviewer: &str,
    args: &[&str],
    env: &[(&str, &str)],
  ) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rust-template-review-cli"))
      .envs(env.iter().copied())
      .current_dir(self.dir())
      // Keep the case away from the developer's own config file and global
      // instructions, so the packet is the case's and nothing else's.
      .env("XDG_CONFIG_HOME", self.work())
      .env("HOME", self.work())
      .arg("--reviewer-cmd")
      .arg(reviewer)
      .args(args)
      .output()
      .expect("the review binary to run")
  }
}

fn stdout(output: &Output) -> String {
  String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
  String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_clean_tree_has_nothing_to_review() {
  let repo = Repo::new();
  let output = repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  assert!(output.status.success());
  assert!(stdout(&output).contains("Nothing to review"));
  assert_eq!(repo.calls(), 0, "the reviewer must not run for nothing");
  assert!(!repo.record_exists(), "no record for an empty change set");
}

#[test]
fn a_clean_verdict_reports_no_findings() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let output = repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  assert!(output.status.success(), "a clean review exits zero");
  assert!(stdout(&output).contains("No findings."));
  assert_eq!(repo.calls(), 1);
}

#[test]
fn findings_are_reported_and_fail_the_run() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let output = repo.review(
    &repo.reviewer(&[finding("src.rs", "comments are sentences")]),
    &["--diff", "head"],
  );
  assert!(!output.status.success(), "a finding must fail the run");
  let report = stdout(&output);
  // The path titles the section and the item carries only the line, so a
  // review of any size does not repeat the path on every entry.
  assert!(report.contains("src.rs"));
  assert!(report.contains("line 1"));
  assert!(report.contains("comments are sentences"));
  assert!(report.contains("llms.org"));
  assert!(report.contains("1 finding."));
}

#[test]
fn an_unchanged_tree_is_not_reviewed_again() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  // A reviewer that would fail if it ran at all proves the record decided.
  let output = repo.review("exit 1", &["--diff", "head"]);
  assert!(output.status.success());
  assert!(stdout(&output).contains("reusing its verdict"));
  assert_eq!(repo.calls(), 1, "the reviewer ran only for the first pass");
}

#[test]
fn only_the_changed_file_reaches_the_packet() {
  let repo = Repo::new();
  repo.write("first.rs", "fn a() {}\n");
  repo.write("second.rs", "fn b() {}\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  repo.write("second.rs", "fn b() { /* changed */ }\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  assert!(packet.contains("second.rs"), "the changed file is judged");
  // The history names every file an earlier round looked at, which is its
  // purpose, so only the changes half of the packet is asserted on here.
  let changes = packet
    .split_once("CHANGES UNDER REVIEW")
    .map(|(_, rest)| rest.to_string())
    .expect("the packet to carry a changes section");
  assert!(
    !changes.contains("first.rs"),
    "a file already judged at this content must not be re-sent:\n{changes}",
  );
}

#[test]
fn a_flagged_file_is_re_reviewed_with_the_batch() {
  let repo = Repo::new();
  repo.write("first.rs", "fn a() {}\n");
  repo.write("second.rs", "fn b() {}\n");
  repo.review(
    &repo.reviewer(&[finding("first.rs", "comments are sentences")]),
    &["--diff", "head"],
  );
  // Only second.rs moved, but first.rs carries a finding whose fix may well
  // live elsewhere, so it must be judged again alongside it.
  repo.write("second.rs", "fn b() { /* changed */ }\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  assert!(packet.contains("first.rs"), "the flagged file rides along");
  assert!(packet.contains("second.rs"));
}

#[test]
fn addressing_a_file_clears_its_findings() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  repo.review(
    &repo.reviewer(&[finding("src.rs", "comments are sentences")]),
    &["--diff", "head"],
  );
  repo.write("src.rs", "// Fixed.\nfn main() {}\n");
  let output = repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  assert!(output.status.success(), "the finding is gone");
  assert!(stdout(&output).contains("No findings."));
}

#[test]
fn the_transcript_reaches_the_reviewer() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  repo.review(
    &repo.reviewer(&[finding("src.rs", "comments are sentences")]),
    &["--diff", "head"],
  );
  repo.write("src.rs", "// Changed.\nfn main() {}\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  assert!(packet.contains("REVIEW HISTORY"));
  assert!(
    packet.contains("comments are sentences"),
    "the reviewer is shown what it already asked for:\n{packet}",
  );
}

#[test]
fn a_finding_outside_the_change_set_is_marked() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let output = repo.review(
    &repo.reviewer(&[finding("untouched.rs", "unrelated")]),
    &["--diff", "head"],
  );
  assert!(!output.status.success());
  assert!(stdout(&output).contains("outside the change set"));
}

#[test]
fn the_record_is_never_part_of_the_review() {
  let repo = Repo::new();
  // No .gitignore at all, so only the in-binary filter can keep the record
  // out of the change set it describes.
  std::fs::remove_file(repo.dir().join(".gitignore")).expect("to remove it");
  repo.commit("drop the ignore file");
  repo.write("src.rs", "fn main() {}\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  repo.write("src.rs", "fn main() { /* again */ }\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  assert!(
    !packet.contains("review.json"),
    "the record must never enter the packet:\n{packet}",
  );
  assert!(repo.record_exists());
}

#[test]
fn auto_reviews_the_working_tree_when_it_is_dirty() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let output = repo.review(&repo.reviewer(&[]), &[]);
  assert!(
    stdout(&output).contains("working tree against HEAD"),
    "unexpected scope: {}",
    stdout(&output),
  );
  assert!(stdout(&output).contains("inferred"), "it names its choice");
}

#[test]
fn auto_reviews_the_branch_when_the_tree_is_clean() {
  let repo = Repo::new();
  repo.git(&["switch", "--create", "feature"]);
  repo.write("src.rs", "fn main() {}\n");
  repo.commit("land the work");
  let output = repo.review(&repo.reviewer(&[]), &[]);
  assert!(
    stdout(&output).contains("branch against"),
    "unexpected scope: {}",
    stdout(&output),
  );
  assert!(
    repo.packet().contains("src.rs"),
    "committed work is still under review in branch mode",
  );
}

#[test]
fn the_two_comparisons_keep_separate_records() {
  let repo = Repo::new();
  repo.git(&["switch", "--create", "feature"]);
  repo.write("src.rs", "fn main() {}\n");
  repo.commit("land the work");
  repo.review(&repo.reviewer(&[]), &["--diff", "branch"]);
  repo.write("other.rs", "fn b() {}\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let record = repo.record();
  let buckets = record["reviews"].as_object().expect("bucketed reviews");
  assert_eq!(buckets.len(), 2, "one bucket per comparison: {buckets:?}");
  assert!(buckets.keys().any(|key| key.starts_with("head:")));
  assert!(buckets.keys().any(|key| key.starts_with("branch:")));
}

#[test]
fn the_llm_prompt_carries_the_findings_inline() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let output = repo.review(
    &repo.reviewer(&[finding("src.rs", "comments are sentences")]),
    &["--diff", "head", "--output", "llm-prompt"],
  );
  let prompt = stdout(&output);
  assert!(prompt.contains("Your task is to satisfy a code review"));
  assert!(prompt.contains("src.rs"));
  assert!(prompt.contains("line 1"));
  assert!(
    prompt.contains("comments are sentences"),
    "the violated rule is cited: {prompt}",
  );
  assert!(
    !output.status.success(),
    "the exit code carries the verdict in every output mode",
  );
}

#[test]
fn a_corrupt_record_fails_and_names_the_file() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  std::fs::write(repo.dir().join("review.json"), r#"{"schema":1,"revi"#)
    .expect("to write a truncated record");
  let output = repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  assert_eq!(
    output.status.code(),
    Some(2),
    "a review that could not run must not look like one that found something",
  );
  assert!(
    stderr(&output).contains("review.json"),
    "the error names the file to delete: {}",
    stderr(&output),
  );
}

#[test]
fn the_exit_code_tells_findings_apart_from_failure() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  assert_eq!(
    repo
      .review(
        &repo.reviewer(&[finding("src.rs", "comments are sentences")]),
        &["--diff", "head"],
      )
      .status
      .code(),
    Some(1),
    "a review that ran and found something exits 1",
  );
  // A path the record has not seen, so the reviewer is actually consulted
  // again; an unchanged tree would be answered from the record without
  // running it at all.
  repo.write("other.rs", "fn b() {}\n");
  assert_eq!(
    repo.review("exit 3", &["--diff", "head"]).status.code(),
    Some(2),
    "a reviewer that could not run exits 2",
  );
}

#[test]
fn a_reviewer_that_fails_is_never_a_pass() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let output = repo.review("exit 3", &["--diff", "head"]);
  assert!(!output.status.success());
  assert!(!repo.record_exists(), "a failed review records nothing");
}

#[test]
fn the_packet_carries_the_conventions_as_committed() {
  let repo = Repo::new();
  repo.write("README.org", "#+title: Case\nThe committed copy.\n");
  repo.commit("record a convention");
  repo.write("README.org", "#+title: Case\nThe working copy.\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  assert!(packet.contains("CONVENTIONS: README.org"));
  assert!(
    packet.contains("The committed copy."),
    "conventions come from the diff base, not the tree under review:\n{packet}",
  );
}

#[test]
fn each_format_writes_its_own_markup() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let reviewer = repo.reviewer(&[finding("src.rs", "comments are sentences")]);
  let rendered = |format: &str| {
    stdout(&repo.review(&reviewer, &["--diff", "head", "--format", format]))
  };
  assert!(rendered("md").contains("- [ ] `line 1`"), "markdown task list");
  assert!(rendered("org").contains("- [ ] =line 1="), "org checkbox");
  let plain = rendered("plain");
  assert!(!plain.contains('`') && !plain.contains("- [ ]"), "no markup");
}

#[test]
fn a_format_switch_does_not_reuse_the_other_markup() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let reviewer = repo.reviewer(&[finding("src.rs", "comments are sentences")]);
  repo.review(&reviewer, &["--diff", "head", "--format", "md"]);
  repo.review(&reviewer, &["--diff", "head", "--format", "org"]);
  // A finding is cached with its markup baked into the text, so each format
  // keeps its own bucket rather than re-rendering the other's prose.
  let keys: Vec<String> = repo.record()["reviews"]
    .as_object()
    .expect("bucketed reviews")
    .keys()
    .cloned()
    .collect();
  assert!(keys.iter().any(|key| key.ends_with(":md")), "{keys:?}");
  assert!(keys.iter().any(|key| key.ends_with(":org")), "{keys:?}");
}

/// A finding whose prose is long enough that a reflow visibly changes it.
fn wordy(path: &str) -> String {
  format!(
    r#"{{"path":"{path}","line":1,"convention":"A judgement long enough that leaving it on one line would run well past the eightieth column and be hard to read","document":"llms.org","fix":"Shorten it, and keep =a b c= intact across any break the reflow introduces."}}"#
  )
}

#[test]
fn an_org_report_reflows_to_eighty_columns() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let report = stdout(&repo.review(
    &repo.reviewer(&[wordy("src.rs")]),
    &["--diff", "head", "--format", "org", "--column-wrap", "80"],
  ));
  assert!(
    report.lines().all(|line| line.chars().count() <= 80),
    "every line fits the wrap column:\n{report}",
  );
  assert!(
    report.contains("=a b c="),
    "a verbatim span survives the reflow whole:\n{report}",
  );
}

#[test]
fn an_unwrapped_report_is_left_alone() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let report = stdout(&repo.review(
    &repo.reviewer(&[wordy("src.rs")]),
    &["--diff", "head", "--format", "org"],
  ));
  assert!(
    report.lines().any(|line| line.chars().count() > 80),
    "omitting --column-wrap leaves the report unwrapped:\n{report}",
  );
}

#[test]
fn a_wrap_the_report_cannot_serve_is_refused_before_reviewing() {
  let refusals = [
    // org-fmt's wrap column is fixed, so no other width can be honoured.
    vec!["--diff", "head", "--format", "org", "--column-wrap", "100"],
    // org-fmt reads org alone, so no other markup can be reflowed.
    vec!["--diff", "head", "--format", "md", "--column-wrap", "80"],
  ];
  refusals.iter().for_each(|args| {
    let repo = Repo::new();
    repo.write("src.rs", "fn main() {}\n");
    // A reviewer that would fail loudly if it ran at all, so the case proves
    // the refusal costs no review rather than merely reporting one.
    let output = repo.review("exit 9", args);
    assert_eq!(output.status.code(), Some(2), "{args:?}");
    assert!(stderr(&output).contains("--column-wrap"), "{args:?}");
    assert_eq!(repo.calls(), 0, "no reviewer ran for {args:?}");
  });
}

#[test]
fn a_wrapped_report_carries_no_colour() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let report = stdout(&repo.review_env(
    &repo.reviewer(&[wordy("src.rs")]),
    &["--diff", "head", "--format", "org", "--column-wrap", "80"],
    &[("CLICOLOR_FORCE", "1")],
  ));
  // A reflow counts an escape sequence toward the column width, so colour in
  // the text being wrapped makes every line stop short of the wrap column.
  assert!(
    !report.contains('\u{1b}'),
    "a wrapped report must not carry escapes:\n{report:?}",
  );
  assert!(
    report.lines().any(|line| line.chars().count() > 70),
    "lines reach the wrap column rather than stopping short:\n{report}",
  );
}

#[test]
fn a_convention_the_base_lacks_is_skipped_in_any_locale() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  // The stub carries README.org and nothing else, so every other convention
  // path is absent at the base; an absent document is skipped rather than
  // failing the review.  The French locale stays as a guard:
  // should a read of git's translated messages ever creep back, this case
  // goes red wherever fr_FR is installed.
  let output = repo.review_env(
    &repo.reviewer(&[]),
    &["--diff", "head"],
    &[("LC_ALL", "fr_FR.UTF-8"), ("LANG", "fr_FR.UTF-8")],
  );
  assert!(
    output.status.success(),
    "an absent convention document must not fail the review: {}",
    stderr(&output),
  );
  assert!(
    repo.packet().contains("CONVENTIONS: README.org"),
    "the conventions the base does carry still reach the packet",
  );
}

#[test]
fn an_unborn_repository_reviews_everything_against_the_empty_tree() {
  let repo = Repo::unborn();
  repo.write("src.rs", "fn main() {}\n");
  let output = repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  assert!(output.status.success(), "{}", stderr(&output));
  let packet = repo.packet();
  assert!(packet.contains("+++ b/src.rs"), "{packet}");
  assert!(packet.contains("+fn main() {}"), "{packet}");
}

#[test]
fn the_packet_diff_is_a_unified_patch() {
  let repo = Repo::new();
  repo.write("README.org", "#+title: Case\nA second line.\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  [
    "diff --git a/README.org b/README.org",
    "--- a/README.org",
    "+++ b/README.org",
    "@@ -1",
    " #+title: Case",
    "+A second line.",
  ]
  .iter()
  .for_each(|expected| {
    assert!(packet.contains(expected), "missing {expected:?}:\n{packet}");
  });
}

#[test]
fn a_deleted_file_appears_as_a_deletion() {
  let repo = Repo::new();
  std::fs::remove_file(repo.dir().join("README.org")).expect("to delete it");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  assert!(packet.contains("deleted file mode 100644"), "{packet}");
  assert!(packet.contains("+++ /dev/null"), "{packet}");
  assert!(packet.contains("-#+title: Case"), "{packet}");
}

#[test]
fn a_renamed_file_diffs_against_its_origin() {
  let repo = Repo::new();
  // Long enough that the moved file is recognised as the same one.
  let body: String = (1..=40).map(|n| format!("line {n}\n")).collect();
  repo.write("old.txt", &body);
  repo.commit("add the file");
  repo.git(&["mv", "old.txt", "new.txt"]);
  repo.write("new.txt", &body.replace("line 40\n", "line forty\n"));
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  assert!(
    packet.contains("rename from old.txt\nrename to new.txt\n"),
    "{packet}"
  );
  assert!(packet.contains("-line 40\n+line forty\n"), "{packet}");
  assert!(
    !packet.contains("+line 1\n"),
    "a rename shows what changed, not the whole file as new:\n{packet}",
  );
}

#[test]
fn a_binary_file_is_marked_rather_than_dumped() {
  let repo = Repo::new();
  std::fs::write(repo.dir().join("blob.bin"), [0u8, 159, 146, 150, 0, 1, 2])
    .expect("to write the binary");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  assert!(
    packet.contains("Binary files /dev/null and b/blob.bin differ"),
    "{packet}"
  );
  assert!(!packet.contains("@@"), "no hunk for a binary:\n{packet}");
}

#[test]
fn a_non_ascii_path_reaches_the_packet_literally() {
  let repo = Repo::new();
  repo.write("src/日本語.rs", "fn main() {}\n");
  repo.review(&repo.reviewer(&[]), &["--diff", "head"]);
  let packet = repo.packet();
  assert!(packet.contains("+++ b/src/日本語.rs"), "{packet}");
  assert!(packet.contains("UNTRACKED FILE: src/日本語.rs"), "{packet}");
}

#[cfg(unix)]
#[test]
fn the_review_runs_without_git_on_path() {
  let repo = Repo::new();
  repo.write("src.rs", "fn main() {}\n");
  let output = repo.review_env(
    &repo.reviewer(&[]),
    &["--diff", "head"],
    &[("PATH", &repo.path_without_git())],
  );
  assert!(
    output.status.success(),
    "the review must not need a git binary: {}",
    stderr(&output),
  );
  assert_eq!(repo.calls(), 1);
}

#[test]
fn origin_head_names_the_default_branch() {
  // The default branch is named so that neither `main` nor `master` exists
  // anywhere: only `origin/HEAD` can say what the branch left.
  let upstream = Repo::init("trunk");
  upstream.write(".gitignore", "review.json*\n");
  upstream.write("README.org", "#+title: Case\n");
  upstream.commit("initial");
  let repo = upstream.clone_of();
  repo.git(&["switch", "--create", "feature"]);
  repo.write("src.rs", "fn main() {}\n");
  repo.commit("land the work");
  let output = repo.review(&repo.reviewer(&[]), &[]);
  assert!(
    stdout(&output).contains("branch against"),
    "unexpected scope: {}\n{}",
    stdout(&output),
    stderr(&output),
  );
  assert!(repo.packet().contains("+++ b/src.rs"), "{}", repo.packet());
}
