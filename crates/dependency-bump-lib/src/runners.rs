//! Runner-image labels: the versioned `ubuntu-NN.04` (and `-arm`) labels the
//! workflows pin, and the newest generally-available image the
//! actions/runner-images README advertises.
//!
//! GitHub redefines `ubuntu-latest` on its own schedule, so a workflow that
//! floats on it changes image under every branch at once with no PR to show
//! for it.  The workflows pin a versioned label instead, and this module
//! plans the pin's advance from the labels in the workflow text and the
//! README's "Available Images" table.  The fetch and the file writes live in
//! `run`; everything here is pure.

use std::fmt;
use std::ops::Range;
use std::path::PathBuf;
use std::process::ExitStatus;
use thiserror::Error;

/// Why the runner-image lookup produced nothing; feeds the caller's warning
/// and never stops the run.
#[derive(Debug, Error)]
pub enum RunnerImageProbeError {
  #[error("could not run curl to fetch the runner-images README: {0}")]
  Spawn(#[from] std::io::Error),
  #[error(
    "curl could not fetch the runner-images README (exited with {status}): \
     {stderr}"
  )]
  Fetch { status: ExitStatus, stderr: String },
  #[error("the runner-images README is not UTF-8: {0}")]
  Encoding(#[from] std::string::FromUtf8Error),
  #[error(
    "the runner-images README lists no generally-available ubuntu image; \
     its \"Available Images\" table may have changed shape"
  )]
  NoUbuntuRows,
}

/// Which Ubuntu runner family a label names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
  X64,
  Arm,
}

impl Variant {
  const ALL: [Variant; 2] = [Variant::X64, Variant::Arm];
}

/// The name a bump of the variant is reported under.
impl fmt::Display for Variant {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(match self {
      Variant::X64 => "GitHub-hosted Ubuntu runner image",
      Variant::Arm => "GitHub-hosted Ubuntu arm runner image",
    })
  }
}

/// One versioned Ubuntu runner label, such as `ubuntu-24.04-arm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Label {
  pub version: u32,
  pub variant: Variant,
}

impl fmt::Display for Label {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self.variant {
      Variant::X64 => write!(formatter, "ubuntu-{}", release(self.version)),
      Variant::Arm => write!(formatter, "ubuntu-{}-arm", release(self.version)),
    }
  }
}

/// The Ubuntu release an LTS version number names, spelled as GitHub does.
pub fn release(version: u32) -> String {
  format!("{version}.04")
}

/// One workflow file's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowFile {
  pub path: PathBuf,
  pub text: String,
}

/// The highest version seen per variant: the newest GA image when read from
/// the README, the current pin when read from the workflows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Versions {
  pub x64: Option<u32>,
  pub arm: Option<u32>,
}

impl Versions {
  pub fn is_empty(&self) -> bool {
    self.x64.is_none() && self.arm.is_none()
  }

  fn get(&self, variant: Variant) -> Option<u32> {
    match variant {
      Variant::X64 => self.x64,
      Variant::Arm => self.arm,
    }
  }

  fn with(self, label: Label) -> Self {
    match label.variant {
      Variant::X64 => Self {
        x64: self.x64.max(Some(label.version)),
        ..self
      },
      Variant::Arm => Self {
        arm: self.arm.max(Some(label.version)),
        ..self
      },
    }
  }
}

/// One planned advance of a variant's pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunnerBump {
  pub variant: Variant,
  pub from: u32,
  pub to: u32,
}

fn is_label_char(character: char) -> bool {
  character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
}

/// Every versioned Ubuntu label token in `text`, with its byte span.
///
/// The README's Included Software cell carries link references such as
/// `[ubuntu-24.04-arm64]`, which a prefix match would read as the arm label
/// with junk after it, so a token counts only where label characters stop
/// on both sides of it.
pub fn labels_in(text: &str) -> Vec<(Range<usize>, Label)> {
  text
    .match_indices("ubuntu-")
    .filter(|(start, _)| {
      !text[..*start]
        .chars()
        .next_back()
        .is_some_and(is_label_char)
    })
    .filter_map(|(start, _)| label_at(text, start))
    .collect()
}

fn label_at(text: &str, start: usize) -> Option<(Range<usize>, Label)> {
  let rest = &text[start + "ubuntu-".len()..];
  let digits = rest.chars().take_while(char::is_ascii_digit).count();
  let version = version_number(&rest[..digits])?;
  let tail = rest[digits..].strip_prefix(".04")?;
  let (variant, tail) = tail
    .strip_prefix("-arm")
    .map_or((Variant::X64, tail), |tail| (Variant::Arm, tail));
  let end = text.len() - tail.len();
  (!tail.chars().next().is_some_and(is_label_char))
    .then_some((start..end, Label { version, variant }))
}

fn version_number(digits: &str) -> Option<u32> {
  (!digits.is_empty()).then_some(digits).and_then(|digits| {
    digits.chars().try_fold(0u32, |total, digit| {
      total.checked_mul(10)?.checked_add(digit.to_digit(10)?)
    })
  })
}

/// The newest generally-available image per variant in the README's
/// "Available Images" table.
///
/// A preview image sits in the same table as the GA ones, told apart only
/// by a `preview` badge in its Image cell (Ubuntu 26.04 carried one from
/// June to September 2026), so a row whose Image cell mentions a preview or
/// a beta contributes nothing.
pub fn newest_available(
  readme: &str,
) -> Result<Versions, RunnerImageProbeError> {
  let versions = newest(readme.lines().flat_map(row_labels));
  (!versions.is_empty())
    .then_some(versions)
    .ok_or(RunnerImageProbeError::NoUbuntuRows)
}

/// The labels a GA table row advertises in its YAML Label cell; nothing for
/// any other line.
fn row_labels(line: &str) -> Vec<Label> {
  let cells = line
    .trim_start()
    .strip_prefix('|')
    .map(|row| row.split('|').collect::<Vec<_>>())
    .unwrap_or_default();
  let image = cells
    .first()
    .map(|cell| cell.to_ascii_lowercase())
    .unwrap_or_default();
  let generally_available =
    !image.contains("preview") && !image.contains("beta");
  cells
    .get(2)
    .filter(|_| generally_available)
    .map(|cell| {
      labels_in(cell)
        .into_iter()
        .map(|(_, label)| label)
        .collect()
    })
    .unwrap_or_default()
}

fn newest(labels: impl IntoIterator<Item = Label>) -> Versions {
  labels.into_iter().fold(Versions::default(), Versions::with)
}

/// The highest pinned version per variant across the workflows.
pub fn pinned(files: &[WorkflowFile]) -> Versions {
  newest(
    files.iter().flat_map(|file| {
      labels_in(&file.text).into_iter().map(|(_, label)| label)
    }),
  )
}

/// The advances that bring each pinned variant to its newest GA image.  A
/// variant the README no longer lists, or lists only older, keeps its pin.
pub fn plan(pinned: &Versions, available: &Versions) -> Vec<RunnerBump> {
  Variant::ALL
    .into_iter()
    .filter_map(|variant| {
      pinned
        .get(variant)
        .zip(available.get(variant))
        .filter(|(from, to)| to > from)
        .map(|(from, to)| RunnerBump { variant, from, to })
    })
    .collect()
}

/// `text` with every label of the bump's variant older than its target
/// rewritten to the target label; unchanged when there is none.
pub fn rewrite(text: &str, bump: &RunnerBump) -> String {
  let target = Label {
    version: bump.to,
    variant: bump.variant,
  }
  .to_string();
  let (rewritten, cursor) = labels_in(text)
    .into_iter()
    .filter(|(_, label)| {
      label.variant == bump.variant && label.version < bump.to
    })
    .fold(
      (String::with_capacity(text.len()), 0),
      |(mut rewritten, cursor), (span, _)| {
        rewritten.push_str(&text[cursor..span.start]);
        rewritten.push_str(&target);
        (rewritten, span.end)
      },
    );
  rewritten + &text[cursor..]
}

#[cfg(test)]
mod tests {
  use super::*;

  const GA_TABLE: &str = "\
| Image | Architecture | YAML Label | Included Software |
| --------------------|--------------|---------------------|------------------|
| Ubuntu 26.04<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | x64 | `ubuntu-26.04` | [ubuntu-26.04] |
| Ubuntu 26.04 Arm64<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | arm64 | `ubuntu-26.04-arm` | [ubuntu-26.04-arm64] |
| Ubuntu 24.04<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | x64 | `ubuntu-latest` or `ubuntu-24.04` | [ubuntu-24.04] |
| Ubuntu 24.04 Arm64<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | arm64 | `ubuntu-24.04-arm` | [ubuntu-24.04-arm64] |
| Ubuntu 22.04<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | x64 | `ubuntu-22.04` | [ubuntu-22.04] |
| Ubuntu Slim<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | x64 | `ubuntu-slim` | [ubuntu-slim] |
| Windows Server 2025<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | x64 | `windows-latest` or `windows-2025` | [windows-2025] |
";

  const PREVIEW_TABLE: &str = "\
| Image | Architecture | YAML Label | Included Software |
| --------------------|--------------|---------------------|------------------|
| Ubuntu 26.04 ![preview](https://img.shields.io/badge/preview-0969DA?style=flat&logoColor=white)<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | x64 | `ubuntu-26.04` | [ubuntu-26.04] |
| Ubuntu 26.04 Arm64 ![preview](https://img.shields.io/badge/preview-0969DA?style=flat&logoColor=white)<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | arm64 | `ubuntu-26.04-arm` | [ubuntu-26.04-arm64] |
| Ubuntu 24.04<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | x64 | `ubuntu-latest` or `ubuntu-24.04` | [ubuntu-24.04] |
| Ubuntu 24.04 Arm64<br>![Endpoint Badge](https://img.shields.io/endpoint?url=x) | arm64 | `ubuntu-24.04-arm` | [ubuntu-24.04-arm64] |
";

  fn label(version: u32, variant: Variant) -> Label {
    Label { version, variant }
  }

  fn workflow(text: &str) -> WorkflowFile {
    WorkflowFile {
      path: PathBuf::from("ci.yml"),
      text: text.to_string(),
    }
  }

  #[test]
  fn labels_in_finds_whole_tokens_only() {
    let text = "runs-on: ubuntu-24.04\n\
                {\"runner\":\"ubuntu-22.04-arm\"}\n\
                [ubuntu-24.04-arm64] ubuntu-slim xubuntu-22.04 \
                ubuntu-22.04.1 ubuntu-2204 ubuntu-.04";
    let found = labels_in(text)
      .into_iter()
      .map(|(_, label)| label)
      .collect::<Vec<_>>();
    assert_eq!(found, vec![label(24, Variant::X64), label(22, Variant::Arm)]);
  }

  #[test]
  fn labels_in_reports_the_exact_span() {
    let text = "runs-on: ubuntu-24.04-arm # arm";
    let (span, found) = labels_in(text).remove(0);
    assert_eq!(&text[span], "ubuntu-24.04-arm");
    assert_eq!(found, label(24, Variant::Arm));
  }

  #[test]
  fn a_label_renders_as_github_spells_it() {
    assert_eq!(label(26, Variant::X64).to_string(), "ubuntu-26.04");
    assert_eq!(label(26, Variant::Arm).to_string(), "ubuntu-26.04-arm");
  }

  #[test]
  fn the_ga_table_yields_the_newest_per_variant() {
    assert_eq!(
      newest_available(GA_TABLE).unwrap(),
      Versions {
        x64: Some(26),
        arm: Some(26)
      }
    );
  }

  #[test]
  fn a_preview_row_contributes_nothing() {
    assert_eq!(
      newest_available(PREVIEW_TABLE).unwrap(),
      Versions {
        x64: Some(24),
        arm: Some(24)
      }
    );
  }

  #[test]
  fn a_table_without_ubuntu_rows_is_an_error() {
    let table = "| Image | Architecture | YAML Label | Included Software |\n\
                 | Windows Server 2025 | x64 | `windows-2025` | [w] |\n";
    assert!(matches!(
      newest_available(table),
      Err(RunnerImageProbeError::NoUbuntuRows)
    ));
  }

  #[test]
  fn pinned_takes_the_highest_across_files() {
    let files = [
      workflow("runs-on: ubuntu-22.04\nrunner: ubuntu-24.04-arm"),
      workflow("runs-on: ubuntu-24.04"),
    ];
    assert_eq!(
      pinned(&files),
      Versions {
        x64: Some(24),
        arm: Some(24)
      }
    );
  }

  #[test]
  fn unpinned_workflows_pin_nothing() {
    assert!(pinned(&[workflow("runs-on: ubuntu-latest")]).is_empty());
  }

  #[test]
  fn plan_advances_each_variant_independently() {
    let pinned = Versions {
      x64: Some(24),
      arm: Some(24),
    };
    let available = Versions {
      x64: Some(26),
      arm: Some(24),
    };
    assert_eq!(
      plan(&pinned, &available),
      vec![RunnerBump {
        variant: Variant::X64,
        from: 24,
        to: 26
      }]
    );
  }

  #[test]
  fn plan_never_downgrades() {
    let pinned = Versions {
      x64: Some(26),
      arm: None,
    };
    let available = Versions {
      x64: Some(24),
      arm: Some(26),
    };
    assert!(plan(&pinned, &available).is_empty());
  }

  #[test]
  fn rewrite_moves_every_older_label_of_the_variant() {
    let bump = RunnerBump {
      variant: Variant::X64,
      from: 24,
      to: 26,
    };
    let text = "a: ubuntu-22.04\nb: ubuntu-24.04\nc: ubuntu-24.04-arm\n\
                d: [ubuntu-24.04-arm64]\ne: ubuntu-26.04\n";
    assert_eq!(
      rewrite(text, &bump),
      "a: ubuntu-26.04\nb: ubuntu-26.04\nc: ubuntu-24.04-arm\n\
       d: [ubuntu-24.04-arm64]\ne: ubuntu-26.04\n"
    );
  }

  #[test]
  fn rewrite_leaves_unrelated_text_untouched() {
    let bump = RunnerBump {
      variant: Variant::Arm,
      from: 24,
      to: 26,
    };
    let text = "runs-on: ubuntu-24.04\n";
    assert_eq!(rewrite(text, &bump), text);
  }
}
