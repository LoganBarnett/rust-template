//! End-to-end engine tests against throwaway fixture spawns.
//!
//! Each test builds a temporary spawn tree plus a `config.json` that points at
//! it, then runs the real manifest (`compliance-checks.toml`) against it and
//! asserts the per-check outcomes.
//!
//! These tests exercise the *engine* — outcome classification, applicability
//! gating, parallel run, config parsing — on a deliberately minimal fixture.
//! They do not (and cannot) assert that the fixture satisfies every manifest
//! check: that would require reproducing the whole template here.  The "a real
//! emission satisfies every check" guarantee is enforced separately by
//! `test-crate-add.sh`'s `assert_compliant`, which runs the manifest against
//! actual `new-project.sh` output.

// This is test code, where panicking is the desired failure signal.  clippy's
// in-test heuristic exempts `#[test]` bodies but not the free helper functions
// in an integration-test binary, so opt the whole file into the same
// allowances clippy.toml grants test code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rust_template_compliance_lib::{run, RunOptions, Verdict};
use std::path::{Path, PathBuf};

fn write(path: &Path, contents: &str) {
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent).unwrap();
  }
  std::fs::write(path, contents).unwrap();
}

fn repo_root() -> PathBuf {
  // crates/compliance-lib -> repo root.
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn options(root: &Path) -> RunOptions {
  RunOptions {
    config_path: root.join("config.json"),
    manifest_path: repo_root().join("compliance-checks.toml"),
    template_dir: repo_root(),
    filter: None,
  }
}

/// Write a fully-compliant cli spawn under `dir`.
fn write_compliant_spawn(dir: &Path) {
  for file in ["CHANGELOG.org", "tasks.org", ".envrc", "rustfmt.toml"] {
    write(&dir.join(file), "placeholder\n");
  }
  // Cargo.toml must mention the foundation dependency.
  write(
    &dir.join("Cargo.toml"),
    r#"[workspace.dependencies]
rust-template-foundation = { workspace = true }
"#,
  );
  // The flake must reference the foundation input.
  write(
    &dir.join("flake.nix"),
    r#"{ inputs.foundation.url = "github:LoganBarnett/rust-template"; }
"#,
  );
  // flake.lock exists (required); pins skip before reading it (no Cargo.lock).
  write(&dir.join("flake.lock"), "{}\n");
  // CI calls the reusable workflows.
  write(
    &dir.join(".github/workflows/ci.yml"),
    r#"jobs:
  ci:
    uses: LoganBarnett/rust-template/.github/workflows/reusable-ci.yml@main
"#,
  );
  // A cli crate that enables the foundation "cli" feature.
  write(
    &dir.join("crates/app/Cargo.toml"),
    r#"[dependencies]
rust-template-foundation = { workspace = true, features = ["cli"] }
"#,
  );
  // llms.org with the Template compliance section pointing at canonical docs,
  // plus the Persistent memory section.  Natural org formatting (blank line
  // after each heading, a wrapped paragraph) exercises the orgize-backed
  // scanner rather than fixed line positions.
  write(
    &dir.join("llms.org"),
    r#"* Template compliance

The authoritative description lives at
docs/compliance.org in the template.

* Persistent memory

Codify conventions in the repo, not in local memory.

* Capturing plans

Record plans as TODO entries in tasks.org.
"#,
  );
  // Valid provenance that opts out of the stale-literal check and the two
  // devShell-marker checks: this fixture's flake.nix is a non-evaluable stub
  // (no outputs), so `nix eval` of its devShells would fail — those checks are
  // exercised against real emissions by test-crate-add.sh instead.
  write(
    &dir.join("rust-template.json"),
    r#"{
  "template_sync_hashes": ["abc"],
  "compliance-ignores": [
    "no-stale-rust-template-literals",
    "flake-default-devshell-marker",
    "flake-ci-devshell-marker"
  ]
}"#,
  );
}

fn config_for(spawn_name: &str, spawn_dir: &Path, crates: &str) -> String {
  format!(
    r#"{{
  "templateSpawns": {{
    "{spawn_name}": {{
      "dir": "{}",
      "archived": false,
      "args": {{ "crates": "{crates}", "description": "", "public": false }}
    }}
  }}
}}"#,
    spawn_dir.display()
  )
}

fn outcome<'a>(
  report: &'a rust_template_compliance_lib::SpawnReport,
  id: &str,
) -> &'a Verdict {
  &report
    .checks
    .iter()
    .find(|check| check.id == id)
    .unwrap_or_else(|| panic!("no check with id {id}"))
    .outcome
}

#[test]
fn minimal_spawn_yields_expected_outcomes_without_engine_errors() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let spawn_dir = root.join("good");
  write_compliant_spawn(&spawn_dir);
  write(&root.join("config.json"), &config_for("good", &spawn_dir, "cli"));

  let report = run(&options(root)).unwrap();
  let spawn = report.spawns.iter().find(|s| s.project == "good").unwrap();

  // No check may *error*: an Error means the engine itself stumbled (a
  // malformed manifest entry, a parser blowing up) rather than the spawn
  // legitimately failing a requirement.  Fails are expected here — the minimal
  // fixture intentionally omits most of what a real emission ships.
  let errors: Vec<_> = spawn
    .checks
    .iter()
    .filter(|c| matches!(c.outcome, Verdict::Error { .. }))
    .collect();
  assert!(errors.is_empty(), "unexpected engine errors: {errors:#?}");

  // The opted-out check is reported as Ignored, not Pass.
  assert!(matches!(
    outcome(spawn, "no-stale-rust-template-literals"),
    Verdict::Ignored { .. }
  ));
  // The doc section and its mention both pass.
  assert!(matches!(
    outcome(spawn, "llms-template-compliance-section"),
    Verdict::Pass
  ));
  assert!(matches!(
    outcome(spawn, "llms-template-compliance-points-to-canonical-docs"),
    Verdict::Pass
  ));
  // The Persistent memory and Capturing plans sections are present too.
  assert!(matches!(
    outcome(spawn, "llms-persistent-memory-section"),
    Verdict::Pass
  ));
  assert!(matches!(
    outcome(spawn, "llms-capturing-plans-section"),
    Verdict::Pass
  ));
  // The cli feature check applies and passes; the server check skips.
  assert!(matches!(
    outcome(spawn, "cli-foundation-cli-feature"),
    Verdict::Pass
  ));
  assert!(matches!(
    outcome(spawn, "server-foundation-auth-feature"),
    Verdict::Skip { .. }
  ));
  // No Cargo.lock present, so the pin checks skip rather than error.
  assert!(matches!(
    outcome(spawn, "foundation-pins-agree"),
    Verdict::Skip { .. }
  ));
  assert!(matches!(
    outcome(spawn, "foundation-pins-current"),
    Verdict::Skip { .. }
  ));
  // No hook in the stub, so the behavioural suite check skips rather than
  // running (or erroring); the file-present check is what reports the gap.
  assert!(matches!(
    outcome(spawn, "review-gate-hook-passes-suite"),
    Verdict::Skip { .. }
  ));
}

#[test]
fn drifted_spawn_reports_specific_failures() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let spawn_dir = root.join("drifted");
  write_compliant_spawn(&spawn_dir);
  // Drift it: remove the required CHANGELOG and the llms.org section.
  std::fs::remove_file(spawn_dir.join("CHANGELOG.org")).unwrap();
  write(
    &spawn_dir.join("llms.org"),
    r#"* Some other heading

Nothing about compliance here.
"#,
  );
  write(&root.join("config.json"), &config_for("drifted", &spawn_dir, "cli"));

  let report = run(&options(root)).unwrap();
  let spawn = report
    .spawns
    .iter()
    .find(|s| s.project == "drifted")
    .unwrap();

  assert!(report.has_failures());
  assert!(matches!(
    outcome(spawn, "required-file-changelog"),
    Verdict::Fail { .. }
  ));
  assert!(matches!(
    outcome(spawn, "llms-template-compliance-section"),
    Verdict::Fail { .. }
  ));
  // The mention check fails because its section is gone.
  assert!(matches!(
    outcome(spawn, "llms-template-compliance-points-to-canonical-docs"),
    Verdict::Fail { .. }
  ));
}

#[test]
fn archived_and_missing_spawns_are_skipped() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let config = format!(
    r#"{{
  "templateSpawns": {{
    "archived": {{
      "dir": "{}",
      "archived": true,
      "args": {{ "crates": "cli" }}
    }},
    "gone": {{
      "dir": "{}",
      "archived": false,
      "args": {{ "crates": "cli" }}
    }}
  }}
}}"#,
    root.join("archived").display(),
    root.join("does-not-exist").display(),
  );
  write(&root.join("config.json"), &config);

  let report = run(&options(root)).unwrap();
  assert!(!report.has_failures());
  for spawn in &report.spawns {
    assert!(spawn.checks.is_empty(), "skipped spawns run no checks");
  }
}
