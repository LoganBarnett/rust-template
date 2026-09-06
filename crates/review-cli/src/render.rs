//! How a pass is reported: to the person who ran it, or to the session that
//! will act on it.
//!
//! Findings are grouped by file and written as checkbox items, because a
//! review of any size is a list a person works through rather than a
//! paragraph they read once.  Colour goes on through `if_supports_color`,
//! which is what suppresses it when stdout is not a terminal — owo-colors'
//! plain styling methods emit escapes regardless.  That is why one rendering
//! serves both a reader and a session reading the piped output.

use crate::config::{Format, Output};
use crate::error::AppError;
use owo_colors::{AnsiColors, OwoColorize, Stream};
use rust_template_review_lib::{Finding, Markup, Outcome, Standing};
use std::collections::BTreeMap;
use std::io::{self, Write};

/// The only wrap column available.  org-fmt holds it as a private constant, so
/// this is not a preference the report can talk it out of; naming it here
/// keeps the flag's error message and the reflow from drifting apart.
const ORG_WRAP_COLUMN: u32 = 80;

pub fn print(
  format: Format,
  output: Output,
  columns: u32,
  outcome: &Outcome,
) -> Result<(), AppError> {
  // A reflow measures an escape sequence as visible width and so wraps short.
  // A wrapped report is a document to save or paste rather than something to
  // read in place, so it is rendered without colour instead.
  if columns != 0 {
    owo_colors::set_override(false);
  }
  let markup = Markup::from(format);
  let report = match output {
    Output::Human => human(markup, outcome),
    Output::LlmPrompt => llm_prompt(markup, outcome),
  };
  write!(io::stdout().lock(), "{}", reflowed(format, columns, report)?)
    .map_err(AppError::from)
}

/// Whether the report can be produced as asked, naming what stands in the way
/// when it cannot.
pub fn check(format: Format, columns: u32) -> Result<(), AppError> {
  if columns == 0 {
    Ok(())
  } else if format != Format::Org {
    Err(AppError::WrapNeedsOrg)
  } else if columns != ORG_WRAP_COLUMN {
    Err(AppError::WrapWidthUnavailable {
      asked: columns,
      only: ORG_WRAP_COLUMN,
    })
  } else {
    Ok(())
  }
}

/// The report, reflowed when asked.
fn reflowed(
  format: Format,
  columns: u32,
  report: String,
) -> Result<String, AppError> {
  check(format, columns).map(|()| match columns {
    0 => report,
    _ => org_fmt_lib::format::format_org(&report),
  })
}

/// Whether the pass found anything.
pub fn found_anything(outcome: &Outcome) -> bool {
  match outcome {
    Outcome::Unchanged { .. } => false,
    Outcome::Reviewed { standing, .. } => !standing.passes(),
  }
}

/// Colour `text`, but only when stdout is something that renders colour.
fn paint(text: &str, hue: AnsiColors) -> String {
  text
    .if_supports_color(Stream::Stdout, |painted| painted.color(hue))
    .to_string()
}

fn bold(text: &str) -> String {
  text
    .if_supports_color(Stream::Stdout, OwoColorize::bold)
    .to_string()
}

fn human(markup: Markup, outcome: &Outcome) -> String {
  match outcome {
    Outcome::Unchanged { base } => {
      format!("Nothing to review: {}.\n", base.describe())
    }
    Outcome::Reviewed {
      base,
      standing,
      reviewed,
    } => format!(
      "{}\n\n{}\n{}",
      bold(&markup.heading(1, &format!("Review — {}", base.describe()))),
      pass_note(reviewed),
      if standing.passes() {
        format!("\n{}\n", paint("No findings.", AnsiColors::Green))
      } else {
        format!(
          "\n{}\n\n{}\n",
          grouped(markup, standing).trim_end(),
          tally(standing)
        )
      }
    ),
  }
}

/// Say whether the reviewer ran, so a pass that cost nothing is not mistaken
/// for one that re-read everything.
fn pass_note(reviewed: &[String]) -> String {
  if reviewed.is_empty() {
    "Nothing has changed since the last review; reusing its verdict."
      .to_string()
  } else {
    format!("Judged {} file(s) this pass.", reviewed.len())
  }
}

/// Findings under a heading per file.  One heading and a run of checkboxes
/// reads far better than a flat list repeating the path on every line.
fn grouped(markup: Markup, standing: &Standing) -> String {
  by_file(standing)
    .into_iter()
    .map(|(path, findings)| {
      format!(
        "{}\n\n{}\n",
        bold(&markup.heading(2, &path)),
        findings
          .iter()
          .map(|finding| item(markup, finding))
          .collect::<String>()
      )
    })
    .collect()
}

/// Findings keyed by the file they name, with the ones reported outside the
/// change set gathered under a heading that says so.
fn by_file(standing: &Standing) -> BTreeMap<String, Vec<&Finding>> {
  standing
    .attributed
    .iter()
    .map(|finding| (finding.path.clone(), finding))
    .chain(standing.unscoped.iter().map(|finding| {
      (format!("{} (outside the change set)", finding.path), finding)
    }))
    .fold(BTreeMap::new(), |mut grouped, (path, finding)| {
      grouped.entry(path).or_default().push(finding);
      grouped
    })
}

fn item(markup: Markup, finding: &Finding) -> String {
  format!(
    "{}\n{}\n",
    markup.todo(&format!(
      "{} — {} {}",
      paint(&markup.code(&location(finding)), AnsiColors::Cyan),
      finding.convention,
      // The document is what makes a finding checkable: it names where the
      // rule is written down, so a reader can disagree with it on the merits.
      paint(
        &markup.emphasis(&format!("({})", finding.document)),
        AnsiColors::BrightBlack,
      ),
    )),
    markup.sub_item(&format!(
      "{} {}",
      paint("Fix:", AnsiColors::Green),
      finding.fix
    )),
  )
}

/// Where a finding points, within a section its file already titles.  Line
/// zero means the file as a whole.
fn location(finding: &Finding) -> String {
  if finding.line == 0 {
    "whole file".to_string()
  } else {
    format!("line {}", finding.line)
  }
}

fn tally(standing: &Standing) -> String {
  paint(
    &match standing.count() {
      1 => "1 finding.".to_string(),
      count => format!("{count} findings."),
    },
    AnsiColors::Yellow,
  )
}

/// The prompt for a fresh session.
fn llm_prompt(markup: Markup, outcome: &Outcome) -> String {
  match outcome {
    Outcome::Unchanged { base } => format!(
      "There is nothing to review ({}), so there is nothing to do.\n",
      base.describe()
    ),
    Outcome::Reviewed { base, standing, .. } if standing.passes() => format!(
      "A code review of the current change set ({}) reported no findings.  \
       There is nothing to do.\n",
      base.describe()
    ),
    Outcome::Reviewed { base, standing, .. } => format!(
      "Your task is to satisfy a code review of this repository's current \
       change set.  A review tool judged the changes against the project's \
       conventions and reported the findings below.  Address every one of \
       them.\n\n\
       Scope reviewed: {}\n\n\
       {}\n\
       For each finding, make the smallest change that satisfies the \
       convention it cites.  Do not restructure code the findings do not \
       mention, and do not widen the change set.  If you believe a finding is \
       wrong, make no change for it and say which one and why, rather than \
       working around it.\n\n\
       When you are done, run `just review` again and confirm the findings \
       are gone.\n",
      base.describe(),
      grouped(markup, standing),
    ),
  }
}
