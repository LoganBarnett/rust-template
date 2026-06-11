//! Human-readable rendering of a compliance run.
//!
//! A section per spawn, one line per check (each naming its id so it can be
//! matched to the manifest or a project's `compliance-ignores`), and a final
//! tally.  Coloring is delegated to owo-colors, whose `if_supports_color` gates
//! escapes on real terminal support (`NO_COLOR`, `CLICOLOR`, tty) so piped
//! output stays clean.

use owo_colors::{AnsiColors, OwoColorize, Stream};
use rust_template_compliance_lib::{
  CheckResult, RunReport, SpawnStatus, Verdict,
};

/// Render `report` to stdout.
pub fn print_human(report: &RunReport) {
  let mut checked = 0usize;
  let mut skipped = 0usize;

  for spawn in &report.spawns {
    match spawn.status {
      SpawnStatus::Checked => {
        checked += 1;
        println!("\nChecking: {} ({})", spawn.project, spawn.dir);
        for check in &spawn.checks {
          print_check(check);
        }
      }
      SpawnStatus::ArchivedSkipped => {
        skipped += 1;
        println!(
          "{} {} (archived)",
          paint("SKIP", AnsiColors::Yellow),
          spawn.project
        );
      }
      SpawnStatus::MissingDirSkipped => {
        skipped += 1;
        println!(
          "{} {} (directory missing: {})",
          paint("SKIP", AnsiColors::Yellow),
          spawn.project,
          spawn.dir
        );
      }
    }
  }

  print_summary(report, checked, skipped);
}

fn print_check(check: &CheckResult) {
  let (label, hue, extra) = match &check.outcome {
    Verdict::Pass => ("PASS", AnsiColors::Green, String::new()),
    Verdict::Fail { detail } => {
      ("FAIL", AnsiColors::Red, format!(" — {detail}"))
    }
    Verdict::Skip { reason } => {
      ("SKIP", AnsiColors::Yellow, format!(" ({reason})"))
    }
    Verdict::Ignored { reason } => (
      "IGNORED",
      AnsiColors::Yellow,
      reason
        .as_ref()
        .map_or_else(String::new, |reason| format!(" ({reason})")),
    ),
    Verdict::Error { detail } => {
      ("ERROR", AnsiColors::Red, format!(" — {detail}"))
    }
  };
  println!(
    "  {} [{}] {}{extra}",
    paint(label, hue),
    check.id,
    check.description,
  );
}

fn print_summary(report: &RunReport, checked: usize, skipped: usize) {
  let mut pass = 0usize;
  let mut fail = 0usize;
  let mut skip = 0usize;
  let mut ignored = 0usize;
  let mut error = 0usize;
  for outcome in report.outcomes() {
    match outcome {
      Verdict::Pass => pass += 1,
      Verdict::Fail { .. } => fail += 1,
      Verdict::Skip { .. } => skip += 1,
      Verdict::Ignored { .. } => ignored += 1,
      Verdict::Error { .. } => error += 1,
    }
  }

  println!("\n═══════════════════════════════════════════");
  println!("Projects checked: {checked}  skipped: {skipped}");
  println!(
    "Checks: {}, {}, {skip} skipped, {ignored} ignored, {}",
    paint(&format!("{pass} passed"), AnsiColors::Green),
    paint(&format!("{fail} failed"), AnsiColors::Red),
    paint(&format!("{error} errored"), AnsiColors::Red),
  );
}

/// Color `text` with `hue`, but only when stdout actually supports color.
fn paint(text: &str, hue: AnsiColors) -> String {
  text
    .if_supports_color(Stream::Stdout, |t| t.color(hue))
    .to_string()
}
