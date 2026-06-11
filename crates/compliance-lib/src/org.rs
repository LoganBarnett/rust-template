//! Org-mode scanning for the documentation checks, backed by the orgize parser.
//!
//! Heading and section detection go through a real org parser rather than
//! line-by-line heuristics, so the checks stay correct as documents use more of
//! org's syntax.  For a mention check, the target section's text is
//! reconstructed from its inline elements — including link targets and
//! descriptions — and whitespace-flattened, so a phrase wrapped across lines is
//! still found.

use orgize::elements::Element;
use orgize::{Event, Org};
use std::ops::Range;

/// True when `path` resolves to a heading.  `path` is a section path: a
/// sequence of nested heading titles, outermost first.  A top-level section is
/// a single-element path; `["Upcoming", "Added"]` is the `** Added` subheading
/// nested under `* Upcoming`.
pub fn section_exists(text: &str, path: &[String]) -> bool {
  let org = Org::parse(text);
  let events: Vec<_> = org.iter().collect();
  locate(&events, path).is_some()
}

/// Whether `needle` appears in `text` (optionally scoped to the section at
/// `path`) after the relevant text is reconstructed and whitespace-flattened.
///
/// Returns `Err` with a human reason when a requested `path` does not resolve
/// — distinct from "the section exists but the phrase is absent".
pub fn mention_present(
  text: &str,
  path: Option<&[String]>,
  needle: &str,
) -> Result<bool, String> {
  let org = Org::parse(text);
  let haystack = match path {
    Some(path) => section_text(&org, path)
      .ok_or_else(|| format!("section \"{}\" not found", path.join(" > ")))?,
    None => document_text(&org),
  };
  Ok(flatten_whitespace(&haystack).contains(&flatten_whitespace(needle)))
}

/// The event range of the body of the heading reached by following `path`:
/// every event after that heading up to the next heading at its level or
/// shallower, bounded by the enclosing section.  Each step descends — the next
/// title must match a heading strictly deeper than its parent and lying within
/// the parent's body — so a one-element path matches a heading at any level
/// while a longer path enforces the nesting.  `None` if any step fails.
fn locate(events: &[Event], path: &[String]) -> Option<Range<usize>> {
  let mut lo = 0;
  let mut hi = events.len();
  let mut parent_level = 0;
  for title in path {
    let want = title.trim();
    let pos = lo
      + events[lo..hi].iter().position(|event| {
        matches!(event, Event::Start(Element::Title(t))
          if t.level > parent_level && t.raw.trim() == want)
      })?;
    let Event::Start(Element::Title(heading)) = &events[pos] else {
      return None;
    };
    let level = heading.level;
    let body_start = pos + 1;
    hi = events[body_start..hi]
      .iter()
      .position(|event| {
        matches!(event, Event::Start(Element::Title(t)) if t.level <= level)
      })
      .map_or(hi, |offset| body_start + offset);
    lo = body_start;
    parent_level = level;
  }
  Some(lo..hi)
}

/// Reconstruct the inline text of the section at `path`, including any
/// subsections (headings deeper than the matched one), or `None` if the path
/// does not resolve.
fn section_text(org: &Org, path: &[String]) -> Option<String> {
  let events: Vec<_> = org.iter().collect();
  Some(
    events[locate(&events, path)?]
      .iter()
      .flat_map(event_text)
      .collect::<Vec<_>>()
      .join(" "),
  )
}

/// Reconstruct the inline text of the whole document.
fn document_text(org: &Org) -> String {
  org
    .iter()
    .flat_map(|event| event_text(&event))
    .collect::<Vec<_>>()
    .join(" ")
}

/// The text an event contributes: a heading's title, an inline text run, or a
/// link's target and description (two pieces, hence the `Vec`).  Anything that
/// carries no text yields nothing.
fn event_text(event: &Event) -> Vec<String> {
  match event {
    Event::Start(Element::Title(title)) => vec![title.raw.to_string()],
    Event::Start(Element::Text { value })
    | Event::Start(Element::Verbatim { value })
    | Event::Start(Element::Code { value }) => vec![value.to_string()],
    Event::Start(Element::Link(link)) => link.desc.as_ref().map_or_else(
      || vec![link.path.to_string()],
      |desc| vec![link.path.to_string(), desc.to_string()],
    ),
    _ => Vec::new(),
  }
}

/// Collapse every run of whitespace (including newlines) into a single space.
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
}
