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
  // A cli-role crate under a custom name exercises the role declaration below.
  // The empty config and integration-test stubs satisfy the cli file-present
  // checks through path resolution.
  write(
    &dir.join("crates/app/Cargo.toml"),
    r#"[dependencies]
rust-template-foundation = { workspace = true, features = ["cli"] }
"#,
  );
  write(&dir.join("crates/app/src/config.rs"), "");
  write(&dir.join("crates/app/tests/integration_test.rs"), "");
  // The spawn's llms.org is the template's own, so the paragraph checks compare
  // a copy that cannot drift.
  std::fs::copy(repo_root().join("template/llms.org"), dir.join("llms.org"))
    .unwrap();
  // Valid provenance that declares the custom-named cli-role crate.  It opts
  // out of the stale-literal check and the two devShell-marker checks.  This
  // fixture's flake.nix is a stub with no outputs, so `nix eval` of its
  // devShells would fail.  test-crate-add.sh runs those checks against real
  // emissions instead.
  write(
    &dir.join("rust-template.json"),
    r#"{
  "template_sync_hashes": ["abc"],
  "workspaces": {
    "app": { "release-binary": true, "type": "cli" }
  },
  "compliance-ignores": [
    "no-stale-rust-template-literals",
    "flake-default-devshell-marker",
    "flake-ci-devshell-marker"
  ]
}"#,
  );
}

fn config_for(spawn_name: &str, spawn_dir: &Path) -> String {
  format!(
    r#"{{
  "templateSpawns": {{
    "{spawn_name}": {{
      "dir": "{}",
      "archived": false
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
  write(&root.join("config.json"), &config_for("good", &spawn_dir));

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
  // A copy of the template's llms.org passes its sections and every llms.org
  // paragraph check.
  for id in [
    "llms-template-compliance-section",
    "llms-persistent-memory-section",
    "llms-template-compliance-paragraphs",
    "llms-imports-ordered",
    "llms-working-with-imports-paragraphs",
    "llms-commit-messages-paragraphs",
    "llms-no-unwrap-paragraphs",
    "llms-prose-rules-paragraphs",
    "llms-program-arguments-paragraphs",
    "llms-import-paths-paragraphs",
  ] {
    assert!(matches!(outcome(spawn, id), Verdict::Pass), "{id}");
  }
  // The cli feature check applies and passes, and the server check skips.  The
  // fixture's cli-role crate lives at crates/app, so a pass here proves the
  // declaration carries the gate.
  assert!(matches!(
    outcome(spawn, "cli-foundation-cli-feature"),
    Verdict::Pass
  ));
  assert!(matches!(
    outcome(spawn, "server-foundation-auth-feature"),
    Verdict::Skip { .. }
  ));
  // Gated on the server role, so a cli-only fixture skips them.
  for id in [
    "darwin-module-calls-foundation-helper",
    "nixos-module-calls-foundation-helper",
    "darwin-module-evaluates",
    "nixos-module-evaluates",
  ] {
    assert!(matches!(outcome(spawn, id), Verdict::Skip { .. }), "{id}");
  }
  // These checks name crates/cli paths.  The declaration resolves them to the
  // stubs under crates/app.
  assert!(matches!(outcome(spawn, "cli-config-present"), Verdict::Pass));
  assert!(matches!(
    outcome(spawn, "cli-integration-test-present"),
    Verdict::Pass
  ));
  // Every crates/* member is declared, and every entry carries its type.
  assert!(matches!(outcome(spawn, "provenance-schema-valid"), Verdict::Pass));
  // No Cargo.lock present, so the pin checks skip rather than error.
  assert!(matches!(
    outcome(spawn, "foundation-pins-agree"),
    Verdict::Skip { .. }
  ));
  assert!(matches!(
    outcome(spawn, "foundation-pins-current"),
    Verdict::Skip { .. }
  ));
  // No Claude settings in the stub, so a check that reads a file the spawn
  // lacks reports it as a plain failure, never an error.
  assert!(matches!(
    outcome(spawn, "settings-disables-auto-memory"),
    Verdict::Fail { .. }
  ));
  // The retired Stop hook is absent, which is the whole of what its
  // retirement check asks for.
  assert!(matches!(outcome(spawn, "review-gate-hook-retired"), Verdict::Pass));
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
  write(&root.join("config.json"), &config_for("drifted", &spawn_dir));

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
  // The paragraph check fails because its section is gone.
  assert!(matches!(
    outcome(spawn, "llms-template-compliance-paragraphs"),
    Verdict::Fail { .. }
  ));
}

#[test]
fn undeclared_custom_crate_skips_role_checks_and_fails_the_schema() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let spawn_dir = root.join("undeclared");
  write_compliant_spawn(&spawn_dir);
  // Remove the role declarations.  An empty workspaces map still matches the
  // schema.  The cli-role crate at crates/app is now invisible to the gates
  // and to path resolution.  It is also unlisted, which the schema check
  // reports.
  write(
    &spawn_dir.join("rust-template.json"),
    r#"{
  "template_sync_hashes": ["abc"],
  "compliance-ignores": [
    "no-stale-rust-template-literals",
    "flake-default-devshell-marker",
    "flake-ci-devshell-marker"
  ]
}"#,
  );
  write(&root.join("config.json"), &config_for("undeclared", &spawn_dir));

  let report = run(&options(root)).unwrap();
  let spawn = report
    .spawns
    .iter()
    .find(|s| s.project == "undeclared")
    .unwrap();

  assert!(matches!(
    outcome(spawn, "cli-foundation-cli-feature"),
    Verdict::Skip { .. }
  ));
  assert!(matches!(outcome(spawn, "cli-config-present"), Verdict::Skip { .. }));
  assert!(matches!(
    outcome(spawn, "provenance-schema-valid"),
    Verdict::Fail { .. }
  ));
}

#[test]
fn legacy_provenance_without_types_skips_role_checks_and_fails_the_schema() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let spawn_dir = root.join("legacy");
  write_compliant_spawn(&spawn_dir);
  // A file from before role declarations.  Its workspaces entry has no
  // `type`, so the strict parse fails and the loader degrades the whole file
  // to the default.  That drops compliance-ignores too, so the assertions
  // below do not lean on the fixture's opt-outs.  A directory named after an
  // archetype is not a role signal, so the cli gate skips despite it.
  write(
    &spawn_dir.join("rust-template.json"),
    r#"{
  "template_sync_hashes": ["abc"],
  "workspaces": {
    "cli": { "release-binary": true }
  }
}"#,
  );
  write(
    &spawn_dir.join("crates/cli/Cargo.toml"),
    r#"[dependencies]
rust-template-foundation = { workspace = true, features = ["cli"] }
"#,
  );
  write(&root.join("config.json"), &config_for("legacy", &spawn_dir));

  let report = run(&options(root)).unwrap();
  let spawn = report
    .spawns
    .iter()
    .find(|s| s.project == "legacy")
    .unwrap();

  assert!(matches!(
    outcome(spawn, "cli-foundation-cli-feature"),
    Verdict::Skip { .. }
  ));
  assert!(matches!(
    outcome(spawn, "provenance-schema-valid"),
    Verdict::Fail { .. }
  ));
}

#[test]
fn declared_role_with_a_missing_directory_fails_loudly() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let spawn_dir = root.join("phantom");
  write_compliant_spawn(&spawn_dir);
  // A declared server role has no directory behind it.  The gate counts the
  // role as present, so its checks run and fail instead of silently skipping.
  write(
    &spawn_dir.join("rust-template.json"),
    r#"{
  "template_sync_hashes": ["abc"],
  "workspaces": {
    "app": { "release-binary": true, "type": "cli" },
    "provider": { "release-binary": true, "type": "server" }
  },
  "compliance-ignores": [
    "no-stale-rust-template-literals",
    "flake-default-devshell-marker",
    "flake-ci-devshell-marker"
  ]
}"#,
  );
  write(&root.join("config.json"), &config_for("phantom", &spawn_dir));

  let report = run(&options(root)).unwrap();
  let spawn = report
    .spawns
    .iter()
    .find(|s| s.project == "phantom")
    .unwrap();

  assert!(matches!(
    outcome(spawn, "server-foundation-auth-feature"),
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
      "archived": true
    }},
    "gone": {{
      "dir": "{}",
      "archived": false
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
