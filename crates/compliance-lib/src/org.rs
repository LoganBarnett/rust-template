//! Org-mode scanning for the documentation checks, backed by the orgize
//! parser.
//!
//! Heading and section detection go through a real org parser rather than
//! line-by-line heuristics, so the checks stay correct as documents use more
//! of org's syntax.  For a mention check, the target scope's text is
//! reconstructed from the tree's text-carrying tokens — including link paths
//! and descriptions — and whitespace-flattened, so a phrase wrapped across
//! lines is still found.
//!
//! Built on orgize 0.10's lossless rowan tree: headings nest as real
//! subtrees, so a section's body — subsections included — is exactly the
//! heading node's subtree, and no event-range arithmetic is needed (the
//! pre-0.10 port of this module maintained event index ranges by hand).

use orgize::ast::{Headline, Paragraph};
use orgize::rowan::ast::AstNode;
use orgize::{Org, SyntaxKind, SyntaxNode};

/// True when `path` resolves to a heading.  `path` is a section path: a
/// sequence of nested heading titles, outermost first.  A top-level section
/// is a single-element path; `["Upcoming", "Added"]` is the `** Added`
/// subheading nested under `* Upcoming`.
pub fn section_exists(text: &str, path: &[String]) -> bool {
  scope_node(&Org::parse(text), path).is_some()
}

/// Whether `needle` appears in `text` (optionally scoped to the section at
/// `path`) after the relevant text is reconstructed and
/// whitespace-flattened.
///
/// Returns `Err` with a human reason when a requested `path` does not
/// resolve — distinct from "the section exists but the phrase is absent".
pub fn mention_present(
  text: &str,
  path: Option<&[String]>,
  needle: &str,
) -> Result<bool, String> {
  let org = Org::parse(text);
  let scope = match path {
    Some(path) => scope_node(&org, path)
      .ok_or_else(|| format!("section \"{}\" not found", path.join(" > ")))?,
    None => org.document().syntax().clone(),
  };
  Ok(
    flatten_whitespace(&scope_text(&scope))
      .contains(&flatten_whitespace(needle)),
  )
}

/// Why an ordered-paragraph assertion did not hold.
#[derive(Debug, PartialEq, Eq)]
pub enum ParagraphsMissing {
  /// The requested section path does not resolve — distinct from "the section
  /// exists but lacks the paragraphs".
  SectionNotFound(String),
  /// The entry at `index` (into the requested list) was not found as a
  /// paragraph of its own after the entries before it.
  NotFoundInOrder { index: usize },
}

/// Whether every entry of `wanted` occurs in `text` (optionally scoped to the
/// section at `path`) as a paragraph of its own — the paragraph's whole text,
/// whitespace-flattened, equals the entry — with those paragraphs appearing
/// in the given relative order.  Other paragraphs may sit anywhere around
/// them: the entries are an ordered subsequence of the scope's paragraphs, not
/// its exact contents.
///
/// A paragraph is matched by its whole text, so two candidate lines with no
/// blank line between them form one paragraph and satisfy neither entry.
/// Indentation is not judged here — a paragraph's text is trimmed — because
/// that is a line-level property, and `line-present` is the kind that asserts
/// it.
pub fn paragraphs_in_order(
  text: &str,
  path: Option<&[String]>,
  wanted: &[String],
) -> Result<(), ParagraphsMissing> {
  let org = Org::parse(text);
  let scope = match path {
    Some(path) => scope_node(&org, path)
      .ok_or_else(|| ParagraphsMissing::SectionNotFound(path.join(" > ")))?,
    None => org.document().syntax().clone(),
  };
  let flattened: Vec<String> = wanted
    .iter()
    .map(|entry| flatten_whitespace(entry))
    .collect();
  // Walk the scope's paragraphs in document order, consuming the wanted list
  // from the front each time a paragraph's whole text equals the next entry.
  let consumed = scope
    .descendants()
    .filter_map(Paragraph::cast)
    .map(|paragraph| flatten_whitespace(&paragraph.syntax().text().to_string()))
    .fold(0, |next, paragraph_text| {
      if flattened
        .get(next)
        .is_some_and(|entry| *entry == paragraph_text)
      {
        next + 1
      } else {
        next
      }
    });
  if consumed == flattened.len() {
    Ok(())
  } else {
    Err(ParagraphsMissing::NotFoundInOrder { index: consumed })
  }
}

/// The syntax node of the heading reached by following `path`, or the
/// document node for an empty path.  Each step descends: the next title
/// must match a heading strictly deeper than its parent, and headings nest
/// as subtrees, so searching within the parent's node enforces "inside the
/// parent's body" structurally.  A one-element path matches a heading at
/// any level; a longer path enforces the nesting.  `None` if any step
/// fails.
fn scope_node(org: &Org, path: &[String]) -> Option<SyntaxNode> {
  path
    .iter()
    .try_fold(org.document().syntax().clone(), |scope, title| {
      // `descendants` includes the scope node itself, but the strictly-
      // deeper level requirement excludes it from matching again.
      let parent_level =
        Headline::cast(scope.clone()).map_or(0, |headline| headline.level());
      let want = title.trim();
      scope
        .descendants()
        .filter_map(Headline::cast)
        .find(|headline| {
          headline.level() > parent_level && headline.title_raw().trim() == want
        })
        .map(|headline| headline.syntax().clone())
    })
}

/// Reconstruct the searchable text of a subtree from its text-carrying
/// tokens: plain text runs (which also cover heading titles, verbatim and
/// code bodies, and link descriptions) plus link path tokens, so a link's
/// target stays findable.  Markup marker tokens contribute nothing —
/// matching the pre-0.10 behavior of searching inline values rather than
/// raw source, so a needle may span a markup boundary.
fn scope_text(scope: &SyntaxNode) -> String {
  scope
    .descendants_with_tokens()
    .filter_map(|element| element.into_token())
    .filter(|token| {
      matches!(token.kind(), SyntaxKind::TEXT | SyntaxKind::LINK_PATH)
    })
    .map(|token| token.text().to_string())
    .collect::<Vec<_>>()
    .join(" ")
}

/// Collapse every run of whitespace (including newlines) into a single
/// space.
fn flatten_whitespace(text: &str) -> String {
  text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
  use super::*;

  // Flush-left so headings sit at column zero, as org requires.
  const DOC: &str = r#"
* Top

Intro text.

** History Hygiene

We want to avoid commits that
are essentially "jk here's the rest".

*** Nested

nested body

** Other

other body

* Second top
"#;

  /// Build a section path from string slices, the way the manifest's
  /// `Vec<String>` arrives at these functions.
  fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
  }

  #[test]
  fn finds_real_headings() {
    assert!(section_exists(DOC, &path(&["Top"])));
    assert!(section_exists(DOC, &path(&["History Hygiene"])));
    assert!(!section_exists(DOC, &path(&["istory"])));
  }

  #[test]
  fn nested_path_descends_and_rejects_siblings() {
    // The path descends from a parent into a true subsection.
    assert!(section_exists(DOC, &path(&["Top", "History Hygiene"])));
    assert!(section_exists(DOC, &path(&["Top", "History Hygiene", "Nested"])));
    // "Other" is a sibling of "History Hygiene", not nested under it.
    assert!(!section_exists(DOC, &path(&["History Hygiene", "Other"])));
    // A heading that does not exist at that depth fails.
    assert!(!section_exists(DOC, &path(&["History Hygiene", "Top"])));
  }

  #[test]
  fn mention_matches_across_a_line_wrap() {
    assert!(mention_present(
      DOC,
      Some(&path(&["History Hygiene"])),
      "commits that are essentially"
    )
    .unwrap());
  }

  #[test]
  fn mention_scoped_to_section_includes_subsections_excludes_siblings() {
    // Deeper subsection content is in scope.
    assert!(mention_present(
      DOC,
      Some(&path(&["History Hygiene"])),
      "nested body"
    )
    .unwrap());
    // Sibling-section content is not.
    assert!(!mention_present(
      DOC,
      Some(&path(&["History Hygiene"])),
      "other body"
    )
    .unwrap());
  }

  #[test]
  fn mention_finds_text_inside_a_link_target() {
    let doc = r#"
* Pointers

See [[https://example.com/docs/compliance.org][=docs/compliance.org=]].
"#;
    assert!(mention_present(
      doc,
      Some(&path(&["Pointers"])),
      "docs/compliance.org"
    )
    .unwrap());
  }

  #[test]
  fn mention_missing_section_is_an_error() {
    assert!(
      mention_present(DOC, Some(&path(&["Nonexistent"])), "anything").is_err()
    );
  }

  #[test]
  fn mention_spans_a_markup_boundary() {
    // The needle crosses from a ~code~ run into plain text; marker tokens
    // must not interpose.  This is the behavior raw-source matching would
    // lose, kept on purpose across the 0.10 port.
    let doc = r#"
* Style

Use the ~tap~ crate for chained logging.
"#;
    assert!(mention_present(
      doc,
      Some(&path(&["Style"])),
      "tap crate for chained logging"
    )
    .unwrap());
  }

  /// The shape the template emits: each import a paragraph of its own, in
  /// the required order, inside a dedicated section.
  const IMPORTS_DOC: &str = r#"
* Imports

@README.org

@CONTRIBUTING.org

* Working with imports

Prose about the imports, which mentions @README.org in passing.
"#;

  #[test]
  fn paragraphs_found_in_order_pass() {
    assert_eq!(
      paragraphs_in_order(
        IMPORTS_DOC,
        Some(&path(&["Imports"])),
        &path(&["@README.org", "@CONTRIBUTING.org"])
      ),
      Ok(())
    );
  }

  #[test]
  fn paragraphs_out_of_order_fail_naming_the_entry() {
    // Both paragraphs exist; CONTRIBUTING is consumed first, then README is
    // sought after it and is not there.  The reported index is README's.
    assert_eq!(
      paragraphs_in_order(
        IMPORTS_DOC,
        Some(&path(&["Imports"])),
        &path(&["@CONTRIBUTING.org", "@README.org"])
      ),
      Err(ParagraphsMissing::NotFoundInOrder { index: 1 })
    );
  }

  #[test]
  fn adjacent_lines_form_one_paragraph_and_satisfy_neither_entry() {
    // What org-fmt produces when the blank line between imports is missing:
    // one paragraph whose text is both lines, equal to neither entry.
    let doc = r#"
* Imports

@README.org
@CONTRIBUTING.org
"#;
    assert_eq!(
      paragraphs_in_order(
        doc,
        Some(&path(&["Imports"])),
        &path(&["@README.org", "@CONTRIBUTING.org"])
      ),
      Err(ParagraphsMissing::NotFoundInOrder { index: 0 })
    );
  }

  #[test]
  fn other_paragraphs_around_the_entries_are_allowed() {
    // A spawn's own imports before, between, and after the required two.
    let doc = r#"
* Imports

@docs/architecture.org

@README.org

@docs/glossary.org

@CONTRIBUTING.org

@docs/runbook.org
"#;
    assert_eq!(
      paragraphs_in_order(
        doc,
        Some(&path(&["Imports"])),
        &path(&["@README.org", "@CONTRIBUTING.org"])
      ),
      Ok(())
    );
  }

  #[test]
  fn paragraphs_in_a_sibling_section_do_not_count() {
    // The imports are well-formed paragraphs, but they sit under a sibling
    // heading, and the Imports scope does not reach a sibling.
    let doc = r#"
* Imports

Nothing here yet.

* Elsewhere

@README.org

@CONTRIBUTING.org
"#;
    assert_eq!(
      paragraphs_in_order(
        doc,
        Some(&path(&["Imports"])),
        &path(&["@README.org", "@CONTRIBUTING.org"])
      ),
      Err(ParagraphsMissing::NotFoundInOrder { index: 0 })
    );
  }

  #[test]
  fn a_mention_inside_prose_is_not_a_paragraph_of_its_own() {
    // Whole-paragraph equality: the passing mention in the prose section
    // must not satisfy an unscoped search either.
    let doc = r#"
* Notes

Please read @README.org before anything else.
"#;
    assert_eq!(
      paragraphs_in_order(doc, None, &path(&["@README.org"])),
      Err(ParagraphsMissing::NotFoundInOrder { index: 0 })
    );
  }

  #[test]
  fn paragraphs_missing_section_is_distinct_from_missing_paragraphs() {
    assert_eq!(
      paragraphs_in_order(
        IMPORTS_DOC,
        Some(&path(&["Nonexistent"])),
        &path(&["@README.org"])
      ),
      Err(ParagraphsMissing::SectionNotFound("Nonexistent".to_string()))
    );
  }

  #[test]
  fn indentation_is_not_this_kinds_concern() {
    // An indented import is still a paragraph of its own here; whether the
    // line is exactly `@README.org` is line-present's assertion, kept
    // separate on purpose.
    let doc = r#"
* Imports

  @README.org

@CONTRIBUTING.org
"#;
    assert_eq!(
      paragraphs_in_order(
        doc,
        Some(&path(&["Imports"])),
        &path(&["@README.org", "@CONTRIBUTING.org"])
      ),
      Ok(())
    );
  }
}
