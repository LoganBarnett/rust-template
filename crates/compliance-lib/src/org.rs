//! A deliberately small org-mode scanner — just enough for the doc checks.
//!
//! It recognises headings (lines beginning at column zero with one or more `*`
//! followed by whitespace) and extracts a section's body as the text between a
//! heading and the next heading of equal-or-higher level (so subsections are
//! included).  For mention checks the body is *flattened* — every run of
//! whitespace, including newlines, collapses to a single space — so a phrase
//! that the author wrapped across lines still matches a single-line needle.
//!
//! This is intentionally not a full org parser; pulling in the external
//! `org-fmt` toolchain for substring presence would be far more than the job
//! needs.

/// The heading level (number of leading `*`) if `line` is a heading, else
/// `None`.  Headings must start at column zero and be followed by whitespace,
/// so `*bold*` and indented text are not headings.
fn heading_level(line: &str) -> Option<usize> {
  if !line.starts_with('*') {
    return None;
  }
  let stars = line.len() - line.trim_start_matches('*').len();
  let rest = &line[stars..];
  if rest.starts_with([' ', '\t']) {
    Some(stars)
  } else {
    None
  }
}

/// The trimmed heading text (without the leading stars).
fn heading_title(line: &str) -> &str {
  line.trim_start_matches('*').trim()
}

/// The body of the section whose heading text equals `title`, or `None` if no
/// such heading exists.  The body runs from just after the heading to the next
/// heading of equal-or-higher level.
pub fn section_body(text: &str, title: &str) -> Option<String> {
  let want = title.trim();
  let lines: Vec<&str> = text.lines().collect();
  let mut index = 0;
  while index < lines.len() {
    if let Some(level) = heading_level(lines[index]) {
      if heading_title(lines[index]) == want {
        let start = index + 1;
        let mut end = start;
        while end < lines.len() {
          if heading_level(lines[end]).is_some_and(|deeper| deeper <= level) {
            break;
          }
          end += 1;
        }
        return Some(lines[start..end].join("\n"));
      }
    }
    index += 1;
  }
  None
}

/// True when `text` has a heading whose text equals `title`.
pub fn section_exists(text: &str, title: &str) -> bool {
  section_body(text, title).is_some()
}

/// Collapse every run of whitespace (including newlines) into a single space.
/// This is what lets a wrapped phrase match a single-line needle.
pub fn flatten_whitespace(text: &str) -> String {
  text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `needle` appears in `text` (optionally scoped to `section`) after
/// both sides are whitespace-flattened.
///
/// Returns `Err` with a human reason when a requested `section` does not exist
/// — the caller turns that into a failing outcome distinct from "the section
/// exists but the phrase is absent".
pub fn mention_present(
  text: &str,
  section: Option<&str>,
  needle: &str,
) -> Result<bool, String> {
  let haystack = match section {
    Some(name) => section_body(text, name)
      .ok_or_else(|| format!("section \"{name}\" not found"))?,
    None => text.to_string(),
  };
  let haystack = flatten_whitespace(&haystack);
  let needle = flatten_whitespace(needle);
  Ok(haystack.contains(&needle))
}

#[cfg(test)]
mod tests {
  use super::*;

  const DOC: &str = "\
* Top
intro text
** History Hygiene
We want to avoid commits that
are essentially \"jk here's the rest\".
*** Nested
nested body
** Other
other body
* Second top
";

  #[test]
  fn finds_a_section_and_its_subsections() {
    let body = section_body(DOC, "History Hygiene").unwrap();
    assert!(body.contains("avoid commits"));
    // Stops at the next equal-level heading.
    assert!(!body.contains("other body"));
    // Includes the deeper subsection.
    assert!(body.contains("nested body"));
  }

  #[test]
  fn section_exists_is_exact_on_title() {
    assert!(section_exists(DOC, "Top"));
    assert!(!section_exists(DOC, "Topp"));
    assert!(!section_exists(DOC, "istory"));
  }

  #[test]
  fn mention_matches_across_a_line_wrap() {
    // The phrase is wrapped across two lines in DOC.
    let found = mention_present(
      DOC,
      Some("History Hygiene"),
      "commits that are essentially",
    )
    .unwrap();
    assert!(found);
  }

  #[test]
  fn mention_missing_section_is_an_error() {
    assert!(mention_present(DOC, Some("Nonexistent"), "anything").is_err());
  }

  #[test]
  fn bold_and_indented_lines_are_not_headings() {
    assert_eq!(heading_level("*bold*"), None);
    assert_eq!(heading_level("  ** indented"), None);
    assert_eq!(heading_level("** real"), Some(2));
  }
}
