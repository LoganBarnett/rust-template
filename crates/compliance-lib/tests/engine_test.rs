//! End-to-end engine tests against throwaway fixture spawns.
//!
//! Each test builds a temporary spawn tree plus a `config.json` that points at
//! it, then runs the real manifest (`compliance-checks.toml`) against it and
//! asserts the per-check outcomes.

// This is test code, where panicking is the desired failure signal.  clippy's
// in-test heuristic exempts `#[test]` bodies but not the free helper functions
// in an integration-test binary, so opt the whole file into the same
// allowances clippy.toml grants test code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rust_template_compliance_lib::{run, CheckOutcome, RunOptions};
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
  for file in ["CHANGELOG.org", ".envrc", "rustfmt.toml"] {
    write(&dir.join(file), "placeholder\n");
  }
  // Cargo.toml must mention the foundation dependency.
  write(
        &dir.join("Cargo.toml"),
        "[workspace.dependencies]\nrust-template-foundation = { workspace = true }\n",
    );
  // The flake must reference the foundation input.
  write(
    &dir.join("flake.nix"),
    "{ inputs.foundation.url = \"github:LoganBarnett/rust-template\"; }\n",
  );
  // flake.lock exists (required); pins skip before reading it (no Cargo.lock).
  write(&dir.join("flake.lock"), "{}\n");
  // CI calls the reusable workflows.
  write(
        &dir.join(".github/workflows/ci.yml"),
        "jobs:\n  ci:\n    uses: LoganBarnett/rust-template/.github/workflows/reusable-ci.yml@main\n",
    );
  // A cli crate that enables the foundation "cli" feature.
  write(
        &dir.join("crates/app/Cargo.toml"),
        "[dependencies]\nrust-template-foundation = { workspace = true, features = [\"cli\"] }\n",
    );
  // llms.org with the Template compliance section pointing at canonical docs.
  write(
        &dir.join("llms.org"),
        "* Template compliance\nThe authoritative description lives at\ndocs/compliance.org in the template.\n",
    );
  // Valid provenance that opts out of the stale-literal check.
  write(
        &dir.join("rust-template.json"),
        "{ \"template_sync_hashes\": [\"abc\"], \"compliance-ignores\": [\"no-stale-rust-template-literals\"] }",
    );
}

fn config_for(spawn_name: &str, spawn_dir: &Path, crates: &str) -> String {
  format!(
        "{{ \"templateSpawns\": {{ \"{spawn_name}\": {{ \"dir\": \"{}\", \"archived\": false, \
         \"args\": {{ \"crates\": \"{crates}\", \"description\": \"\", \"public\": false }} }} }} }}",
        spawn_dir.display()
    )
}

fn outcome<'a>(
  report: &'a rust_template_compliance_lib::SpawnReport,
  id: &str,
) -> &'a CheckOutcome {
  &report
    .checks
    .iter()
    .find(|check| check.id == id)
    .unwrap_or_else(|| panic!("no check with id {id}"))
    .outcome
}

#[test]
fn compliant_spawn_has_no_failures() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let spawn_dir = root.join("good");
  write_compliant_spawn(&spawn_dir);
  write(&root.join("config.json"), &config_for("good", &spawn_dir, "cli"));

  let report = run(&options(root)).unwrap();
  let spawn = report.spawns.iter().find(|s| s.project == "good").unwrap();

  let failures: Vec<_> = spawn
    .checks
    .iter()
    .filter(|c| {
      matches!(
        c.outcome,
        CheckOutcome::Fail { .. } | CheckOutcome::Error { .. }
      )
    })
    .collect();
  assert!(failures.is_empty(), "unexpected failures: {failures:#?}");

  // The opted-out check is reported as Ignored, not Pass.
  assert!(matches!(
    outcome(spawn, "no-stale-rust-template-literals"),
    CheckOutcome::Ignored { .. }
  ));
  // The doc section and its mention both pass.
  assert!(matches!(
    outcome(spawn, "llms-template-compliance-section"),
    CheckOutcome::Pass
  ));
  assert!(matches!(
    outcome(spawn, "llms-template-compliance-points-to-canonical-docs"),
    CheckOutcome::Pass
  ));
  // The cli feature check applies and passes; the server check skips.
  assert!(matches!(
    outcome(spawn, "cli-foundation-cli-feature"),
    CheckOutcome::Pass
  ));
  assert!(matches!(
    outcome(spawn, "server-foundation-auth-feature"),
    CheckOutcome::Skip { .. }
  ));
  // No Cargo.lock present, so the pin checks skip rather than error.
  assert!(matches!(
    outcome(spawn, "foundation-pins-agree"),
    CheckOutcome::Skip { .. }
  ));
  assert!(matches!(
    outcome(spawn, "foundation-pins-current"),
    CheckOutcome::Skip { .. }
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
    "* Some other heading\nNothing about compliance here.\n",
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
    CheckOutcome::Fail { .. }
  ));
  assert!(matches!(
    outcome(spawn, "llms-template-compliance-section"),
    CheckOutcome::Fail { .. }
  ));
  // The mention check fails because its section is gone.
  assert!(matches!(
    outcome(spawn, "llms-template-compliance-points-to-canonical-docs"),
    CheckOutcome::Fail { .. }
  ));
}

#[test]
fn archived_and_missing_spawns_are_skipped() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let config = format!(
        "{{ \"templateSpawns\": {{ \
         \"archived\": {{ \"dir\": \"{}\", \"archived\": true, \"args\": {{ \"crates\": \"cli\" }} }}, \
         \"gone\": {{ \"dir\": \"{}\", \"archived\": false, \"args\": {{ \"crates\": \"cli\" }} }} }} }}",
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
