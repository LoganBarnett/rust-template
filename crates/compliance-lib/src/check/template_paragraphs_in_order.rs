//! The `template-paragraphs-in-order` kind.  Every paragraph of a template
//! section appears whole in the spawn's same section, in the same order.

use super::{read_file, FileRead, Verdict};
use crate::org;
use std::path::Path;

/// Eight is a coin flip.  It is enough words to find a paragraph by search.
const OPENING_WORDS: usize = 8;

/// Judge the spawn at `dir` against the template checkout at `template_dir`.
/// No spawn could pass a broken manifest entry, so it is an `Error`.
pub(super) fn template_paragraphs_in_order(
  dir: &Path,
  template_dir: &Path,
  project: &str,
  target: &str,
  section: &[String],
) -> Verdict {
  template_paragraphs(&template_dir.join("template").join(target), section)
    .map_or_else(
      |detail| Verdict::Error { detail },
      |wanted| match read_file(&dir.join(target)) {
        FileRead::Found(text) => org::paragraphs(&text, section).map_or_else(
          |org::SectionNotFound(name)| Verdict::Fail {
            detail: format!("section \"{name}\" not found in {target}"),
          },
          |found| spawn_verdict(&wanted, &found, project, target, section),
        ),
        FileRead::Missing => Verdict::Fail {
          detail: format!("{target} not found"),
        },
        FileRead::Error(detail) => Verdict::Error { detail },
      },
    )
}

/// The paragraphs the template's section holds, or why the check is broken.
fn template_paragraphs(
  path: &Path,
  section: &[String],
) -> Result<Vec<String>, String> {
  match read_file(path) {
    FileRead::Found(text) => org::paragraphs(&text, section)
      .map_err(|org::SectionNotFound(name)| {
        format!("template section \"{name}\" not found in {}", path.display())
      })
      .and_then(|wanted| {
        if wanted.is_empty() {
          Err(format!(
            "template section \"{}\" in {} has no paragraphs to compare",
            section.join(" > "),
            path.display()
          ))
        } else {
          Ok(wanted)
        }
      }),
    FileRead::Missing => Err(format!("no template file at {}", path.display())),
    FileRead::Error(detail) => Err(detail),
  }
}

/// Pass when `wanted` is an ordered subsequence of `found`.  Otherwise fail on
/// the first template paragraph that the spawn does not hold in order.
fn spawn_verdict(
  wanted: &[String],
  found: &[String],
  project: &str,
  target: &str,
  section: &[String],
) -> Verdict {
  let matched = found.iter().fold(0, |next, paragraph| {
    if wanted
      .get(next)
      .is_some_and(|want| same_paragraph(want, paragraph, project))
    {
      next + 1
    } else {
      next
    }
  });
  wanted
    .get(matched)
    .map_or(Verdict::Pass, |unmatched| Verdict::Fail {
      detail: format!(
        "paragraph {} of {} in template/{target} § {} {} {target}: \"{}\"",
        matched + 1,
        wanted.len(),
        section.join(" > "),
        // Matching is greedy, so a paragraph found anywhere sits before the
        // paragraphs it should follow.
        if found
          .iter()
          .any(|paragraph| same_paragraph(unmatched, paragraph, project))
        {
          format!("does not follow paragraph {matched} in")
        } else {
          "is missing from".to_string()
        },
        opening(unmatched),
      ),
    })
}

/// Whether spawn paragraph `found` is template paragraph `wanted`.  The two
/// may differ by the project-name substitution `new-project.sh` makes.  A
/// project name that also occurs as an ordinary word in the paragraph defeats
/// the reversal.  That takes a name like `lib`, so it is accepted.
fn same_paragraph(wanted: &str, found: &str, project: &str) -> bool {
  wanted == found || wanted == unrenamed(found, project)
}

/// `text` with the project name turned back into `rust-template`, and its
/// underscore form into `rust_template`.
fn unrenamed(text: &str, project: &str) -> String {
  text
    .replace(project, "rust-template")
    .replace(&project.replace('-', "_"), "rust_template")
}

/// The first words of `paragraph`, marked when cut short.
fn opening(paragraph: &str) -> String {
  let opening = paragraph
    .split_whitespace()
    .take(OPENING_WORDS)
    .collect::<Vec<_>>()
    .join(" ");
  if opening.len() < paragraph.len() {
    format!("{opening} ...")
  } else {
    opening
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const TEMPLATE: &str = r#"
* Rules

First rule.

Second rule, from rust-template.

** Detail

Third rule.

* Other

Unchecked.

* Empty

#+begin_src sh
echo no paragraphs here
#+end_src
"#;

  /// Run the check with `TEMPLATE` as the template's `llms.org` and `spawn` as
  /// the spawn's, scoped to `section`.
  fn verdict(spawn: Option<&str>, project: &str, section: &[&str]) -> Verdict {
    let root = tempfile::tempdir().unwrap();
    let checkout = root.path().join("checkout");
    let spawn_dir = root.path().join("spawn");
    std::fs::create_dir_all(checkout.join("template")).unwrap();
    std::fs::create_dir_all(&spawn_dir).unwrap();
    std::fs::write(checkout.join("template/llms.org"), TEMPLATE).unwrap();
    if let Some(spawn) = spawn {
      std::fs::write(spawn_dir.join("llms.org"), spawn).unwrap();
    }
    template_paragraphs_in_order(
      &spawn_dir,
      &checkout,
      project,
      "llms.org",
      &section
        .iter()
        .map(|part| part.to_string())
        .collect::<Vec<_>>(),
    )
  }

  fn rules(spawn: &str, project: &str) -> Verdict {
    verdict(Some(spawn), project, &["Rules"])
  }

  fn fail_detail(verdict: Verdict) -> String {
    match verdict {
      Verdict::Fail { detail } => detail,
      other => panic!("expected a failure, got {other:?}"),
    }
  }

  #[test]
  fn an_exact_copy_passes() {
    assert!(matches!(rules(TEMPLATE, "demo"), Verdict::Pass));
  }

  #[test]
  fn a_reflowed_copy_passes() {
    let spawn = r#"
* Rules

First
rule.

Second rule,
from rust-template.

** Detail

Third  rule.
"#;
    assert!(matches!(rules(spawn, "demo"), Verdict::Pass));
  }

  #[test]
  fn spawn_paragraphs_around_the_template_ones_pass() {
    let spawn = r#"
* Rules

Our own preface.

First rule.

Our own aside.

Second rule, from rust-template.

** Detail

Third rule.

** Our own subsection

Our own detail.
"#;
    assert!(matches!(rules(spawn, "demo"), Verdict::Pass));
  }

  #[test]
  fn a_renamed_project_passes() {
    // new-project.sh wrote the project name over rust-template.
    let spawn = r#"
* Rules

First rule.

Second rule, from my-demo.

** Detail

Third rule.
"#;
    assert!(matches!(rules(spawn, "my-demo"), Verdict::Pass));
  }

  #[test]
  fn the_underscore_form_is_renamed_too() {
    let root = tempfile::tempdir().unwrap();
    let checkout = root.path().join("checkout");
    std::fs::create_dir_all(checkout.join("template")).unwrap();
    std::fs::write(
      checkout.join("template/llms.org"),
      "* Crates\n\nImport rust_template_lib from rust-template.\n",
    )
    .unwrap();
    std::fs::write(
      root.path().join("llms.org"),
      "* Crates\n\nImport my_demo_lib from my-demo.\n",
    )
    .unwrap();
    assert!(matches!(
      template_paragraphs_in_order(
        root.path(),
        &checkout,
        "my-demo",
        "llms.org",
        &["Crates".to_string()],
      ),
      Verdict::Pass
    ));
  }

  #[test]
  fn a_changed_sentence_fails_the_whole_paragraph() {
    let spawn = r#"
* Rules

First rule.

Second rule, from rust-template.  Plus a sentence of our own.

** Detail

Third rule.
"#;
    let detail = fail_detail(rules(spawn, "demo"));
    assert!(detail.contains("paragraph 2 of 3"), "{detail}");
    assert!(detail.contains("is missing from llms.org"), "{detail}");
    assert!(detail.contains("Second rule, from rust-template."), "{detail}");
  }

  #[test]
  fn swapped_paragraphs_fail_as_out_of_order() {
    let spawn = r#"
* Rules

Second rule, from rust-template.

First rule.

** Detail

Third rule.
"#;
    let detail = fail_detail(rules(spawn, "demo"));
    assert!(detail.contains("paragraph 2 of 3"), "{detail}");
    assert!(detail.contains("does not follow paragraph 1"), "{detail}");
  }

  #[test]
  fn a_paragraph_moved_out_of_the_section_fails() {
    let spawn = r#"
* Rules

First rule.

Second rule, from rust-template.

* Other

Third rule.
"#;
    let detail = fail_detail(rules(spawn, "demo"));
    assert!(detail.contains("paragraph 3 of 3"), "{detail}");
    assert!(detail.contains("is missing from"), "{detail}");
  }

  #[test]
  fn a_long_paragraph_is_named_by_its_opening_words() {
    let root = tempfile::tempdir().unwrap();
    let checkout = root.path().join("checkout");
    std::fs::create_dir_all(checkout.join("template")).unwrap();
    std::fs::write(
      checkout.join("template/llms.org"),
      "* Long\n\none two three four five six seven eight nine ten\n",
    )
    .unwrap();
    std::fs::write(root.path().join("llms.org"), "* Long\n\nother\n").unwrap();
    let detail = fail_detail(template_paragraphs_in_order(
      root.path(),
      &checkout,
      "demo",
      "llms.org",
      &["Long".to_string()],
    ));
    assert!(
      detail.ends_with("\"one two three four five six seven eight ...\""),
      "{detail}"
    );
  }

  #[test]
  fn a_missing_spawn_section_fails() {
    let detail = fail_detail(rules("* Other\n\nUnchecked.\n", "demo"));
    assert!(detail.contains("section \"Rules\" not found"), "{detail}");
  }

  #[test]
  fn a_missing_spawn_file_fails() {
    let detail = fail_detail(verdict(None, "demo", &["Rules"]));
    assert!(detail.contains("llms.org not found"), "{detail}");
  }

  #[test]
  fn a_missing_template_section_is_an_error() {
    // The manifest names a section the template lacks, so the check is broken
    // whatever the spawn holds.
    assert!(matches!(
      verdict(Some(TEMPLATE), "demo", &["Nonexistent"]),
      Verdict::Error { .. }
    ));
  }

  #[test]
  fn a_template_section_without_paragraphs_is_an_error() {
    // A check that compares nothing would pass every spawn.
    assert!(matches!(
      verdict(Some(TEMPLATE), "demo", &["Empty"]),
      Verdict::Error { .. }
    ));
  }

  #[test]
  fn a_missing_template_file_is_an_error() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("llms.org"), TEMPLATE).unwrap();
    assert!(matches!(
      template_paragraphs_in_order(
        root.path(),
        &root.path().join("no-checkout"),
        "demo",
        "llms.org",
        &["Rules".to_string()],
      ),
      Verdict::Error { .. }
    ));
  }
}
