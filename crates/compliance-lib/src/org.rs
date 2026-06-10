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

/// True when `text` has a heading whose title equals `title`.
pub fn section_exists(text: &str, title: &str) -> bool {
  let want = title.trim();
  Org::parse(text)
        .iter()
        .any(|event| matches!(event, Event::Start(Element::Title(t)) if t.raw.trim() == want))
}

/// Whether `needle` appears in `text` (optionally scoped to `section`) after
/// the relevant text is reconstructed and whitespace-flattened.
///
/// Returns `Err` with a human reason when a requested `section` does not exist
/// — distinct from "the section exists but the phrase is absent".
pub fn mention_present(
  text: &str,
  section: Option<&str>,
  needle: &str,
) -> Result<bool, String> {
  let org = Org::parse(text);
  let haystack = match section {
    Some(name) => section_text(&org, name.trim())
      .ok_or_else(|| format!("section \"{name}\" not found"))?,
    None => document_text(&org),
  };
  Ok(flatten_whitespace(&haystack).contains(&flatten_whitespace(needle)))
}

/// Reconstruct the inline text of the section whose title equals `want`,
/// including any subsections (headings deeper than the matched one), or `None`
/// if no such heading exists.  The section spans from just after its heading to
/// the next heading at the same level or shallower.
fn section_text(org: &Org, want: &str) -> Option<String> {
  let events: Vec<_> = org.iter().collect();
  let start = events.iter().position(|event| {
    matches!(event, Event::Start(Element::Title(t)) if t.raw.trim() == want)
  })?;
  let Event::Start(Element::Title(heading)) = &events[start] else {
    return None;
  };
  let level = heading.level;
  let end = events[start + 1..]
    .iter()
    .position(|event| {
      matches!(event, Event::Start(Element::Title(t)) if t.level <= level)
    })
    .map_or(events.len(), |offset| start + 1 + offset);
  Some(
    events[start + 1..end]
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

  #[test]
  fn finds_real_headings() {
    assert!(section_exists(DOC, "Top"));
    assert!(section_exists(DOC, "History Hygiene"));
    assert!(!section_exists(DOC, "istory"));
  }

  #[test]
  fn mention_matches_across_a_line_wrap() {
    assert!(mention_present(
      DOC,
      Some("History Hygiene"),
      "commits that are essentially"
    )
    .unwrap());
  }

  #[test]
  fn mention_scoped_to_section_includes_subsections_excludes_siblings() {
    // Deeper subsection content is in scope.
    assert!(
      mention_present(DOC, Some("History Hygiene"), "nested body").unwrap()
    );
    // Sibling-section content is not.
    assert!(
      !mention_present(DOC, Some("History Hygiene"), "other body").unwrap()
    );
  }

  #[test]
  fn mention_finds_text_inside_a_link_target() {
    let doc = r#"
* Pointers

See [[https://example.com/docs/compliance.org][=docs/compliance.org=]].
"#;
    assert!(
      mention_present(doc, Some("Pointers"), "docs/compliance.org").unwrap()
    );
  }

  #[test]
  fn mention_missing_section_is_an_error() {
    assert!(mention_present(DOC, Some("Nonexistent"), "anything").is_err());
  }
}
