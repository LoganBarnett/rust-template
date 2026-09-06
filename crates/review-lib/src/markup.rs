//! The markup a review is written in.
//!
//! It reaches two places that must agree.  The reviewer is told which markup
//! to write its prose in, so a finding arrives already marked up rather than
//! guessed at afterwards; and the record keys on it, because a finding is
//! cached with that markup baked into its text and re-rendering it in another
//! is not something the renderer can undo.

use serde::{Deserialize, Serialize};

#[derive(
  Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Markup {
  /// Prose with no markup, for a terminal or a grep.
  Plain,
  /// Light Markdown: the widest-travelling of the three.
  #[default]
  Markdown,
  /// Light Org, for a project that keeps its documents in it.
  Org,
}

impl Markup {
  /// The name this markup carries in a record key.
  pub fn label(self) -> &'static str {
    match self {
      Self::Plain => "plain",
      Self::Markdown => "md",
      Self::Org => "org",
    }
  }

  /// `text` as a code span.
  pub fn code(self, text: &str) -> String {
    match self {
      Self::Plain => text.to_string(),
      Self::Markdown => format!("`{text}`"),
      Self::Org => format!("={text}="),
    }
  }

  /// `text` emphasised.
  pub fn emphasis(self, text: &str) -> String {
    match self {
      Self::Plain => text.to_string(),
      Self::Markdown => format!("*{text}*"),
      Self::Org => format!("/{text}/"),
    }
  }

  /// A heading at `depth`, counting from one.
  pub fn heading(self, depth: usize, text: &str) -> String {
    match self {
      Self::Plain => text.to_string(),
      Self::Markdown => format!("{} {text}", "#".repeat(depth)),
      Self::Org => format!("{} {text}", "*".repeat(depth)),
    }
  }

  /// An unticked checkbox item.  Org has these natively; Markdown's are a
  /// GitHub extension rather than part of the standard, which is the closest
  /// equivalent available.
  pub fn todo(self, text: &str) -> String {
    match self {
      Self::Plain => format!("  {text}"),
      Self::Markdown | Self::Org => format!("- [ ] {text}"),
    }
  }

  /// An item nested under a checkbox.  The fix belongs to the judgement above
  /// it rather than standing beside it, and nesting is what says so — a reader
  /// ticking off findings sees one box per judgement, not two lines that might
  /// be two.
  pub fn sub_item(self, text: &str) -> String {
    match self {
      Self::Plain => format!("    {text}"),
      Self::Markdown | Self::Org => format!("  - {text}"),
    }
  }

  /// How the reviewer is told to write its prose, appended to the system
  /// prompt so a finding arrives in the markup the report will render.
  pub fn instruction(self) -> &'static str {
    match self {
      Self::Plain => {
        "\n## Markup\n\nWrite the convention and fix fields as plain prose \
         with no markup of any kind.  Name code and paths without quoting or \
         decorating them.\n"
      }
      Self::Markdown => {
        "\n## Markup\n\nWrite the convention and fix fields in light \
         Markdown.  Use backticks around code, identifiers, and paths, and \
         nothing else — no headings, no lists, no links.  The report supplies \
         its own structure; yours is one sentence per field.\n"
      }
      Self::Org => {
        "\n## Markup\n\nWrite the convention and fix fields in light Org \
         markup.  Use =verbatim= around code, identifiers, and paths, and \
         nothing else — no headings, no lists, no links.  The report supplies \
         its own structure; yours is one sentence per field.\n"
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn each_markup_keys_the_record_differently() {
    assert_ne!(Markup::Markdown.label(), Markup::Org.label());
    assert_ne!(Markup::Plain.label(), Markup::Markdown.label());
  }

  #[test]
  fn code_spans_follow_the_markup() {
    assert_eq!(Markup::Markdown.code("x"), "`x`");
    assert_eq!(Markup::Org.code("x"), "=x=");
    assert_eq!(Markup::Plain.code("x"), "x");
  }

  #[test]
  fn both_document_markups_offer_a_checkbox() {
    assert!(Markup::Org.todo("a").starts_with("- [ ]"));
    assert!(Markup::Markdown.todo("a").starts_with("- [ ]"));
  }
}
