//! The `nix-call-present` kind: a call to a named function somewhere in the
//! spawn's Nix, read from the parsed syntax tree rather than from its text.

use super::{read_file, relative_display, walk_files, FileRead, Verdict};
use rowan::ast::AstNode as _;
use std::ffi::OsStr;
use std::path::Path;

/// Read and parse `target` as Nix, applying `check` to the syntax tree.  A
/// missing target fails, as does one that does not parse.
fn with_nix(
  dir: &Path,
  target: &str,
  check: impl FnOnce(&rnix::SyntaxNode) -> Verdict,
) -> Verdict {
  match read_file(&dir.join(target)) {
    FileRead::Found(text) => nix_syntax_tree(&text).map_or_else(
      |error| Verdict::Fail {
        detail: format!("{target}: invalid Nix: {error}"),
      },
      |root| check(&root),
    ),
    FileRead::Missing => Verdict::Fail {
      detail: format!("{target} not present"),
    },
    FileRead::Error(detail) => Verdict::Error { detail },
  }
}

/// The syntax tree of `text`, or its first parse error.  rnix parses
/// tolerantly and still yields a tree for broken input, so the error has to
/// be asked for rather than assumed from the tree's presence.
fn nix_syntax_tree(
  text: &str,
) -> Result<rnix::SyntaxNode, rnix::parser::ParseError> {
  rnix::Root::parse(text)
    .ok()
    .map(|root| root.syntax().clone())
}

/// Whether the tree holds a call whose callee resolves to `function`.  A
/// curried `f a b` is found without peeling: every `Apply` node is visited,
/// and the innermost application's callee is `f` itself.
fn nix_calls(root: &rnix::SyntaxNode, function: &str) -> bool {
  root
    .descendants()
    .filter_map(rnix::ast::Apply::cast)
    .filter_map(|apply| apply.lambda())
    .filter_map(callee_name)
    .any(|name| name == function)
}

/// The name a callee expression resolves to: a bare identifier, the last
/// segment of an attribute select, or either wrapped in parentheses.  A
/// dynamic or quoted last segment resolves to nothing.
fn callee_name(expr: rnix::ast::Expr) -> Option<String> {
  use rnix::ast::{Attr, Expr};
  match expr {
    Expr::Paren(paren) => paren.expr().and_then(callee_name),
    Expr::Ident(ident) => ident_text(&ident),
    Expr::Select(select) => select
      .attrpath()
      .and_then(|path| path.attrs().last())
      .and_then(|attr| match attr {
        Attr::Ident(ident) => ident_text(&ident),
        _ => None,
      }),
    _ => None,
  }
}

fn ident_text(ident: &rnix::ast::Ident) -> Option<String> {
  ident.ident_token().map(|token| token.text().to_string())
}

/// Pass when `function` is called in `target` or, with no target, in any
/// `.nix` file under `dir`.
pub(super) fn nix_call_present(
  dir: &Path,
  function: &str,
  target: Option<&str>,
) -> Verdict {
  target.map_or_else(
    || nix_call_in_tree(dir, function),
    |target| {
      with_nix(dir, target, |root| {
        if nix_calls(root, function) {
          Verdict::Pass
        } else {
          Verdict::Fail {
            detail: format!("{target}: no call to `{function}`"),
          }
        }
      })
    },
  )
}

/// Pass when any `.nix` file under `dir` calls `function`; fail naming the
/// first unparseable file, else the count scanned.
fn nix_call_in_tree(dir: &Path, function: &str) -> Verdict {
  walk_files(dir)
    .into_iter()
    .filter(|path| path.extension() == Some(OsStr::new("nix")))
    // `Err` ends the scan early with its verdict.
    .try_fold(0usize, |scanned, path| match read_file(&path) {
      FileRead::Found(text) => match nix_syntax_tree(&text) {
        Ok(root) if nix_calls(&root, function) => Err(Verdict::Pass),
        Ok(_) => Ok(scanned + 1),
        Err(error) => Err(Verdict::Fail {
          detail: format!(
            "{}: invalid Nix: {error}",
            relative_display(dir, &path)
          ),
        }),
      },
      FileRead::Missing => Ok(scanned),
      FileRead::Error(detail) => Err(Verdict::Error { detail }),
    })
    .map_or_else(
      |verdict| verdict,
      |scanned| Verdict::Fail {
        detail: format!("no .nix file calls `{function}` ({scanned} scanned)"),
      },
    )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn calls(source: &str) -> bool {
    nix_calls(&nix_syntax_tree(source).unwrap(), "mkDarwinService")
  }

  #[test]
  fn nix_calls_sees_a_select_callee() {
    assert!(calls("foundation.lib.mkDarwinService { name = \"x\"; }"));
  }

  #[test]
  fn nix_calls_sees_a_bare_identifier_callee() {
    assert!(calls(
      "let inherit (foundation.lib) mkDarwinService; in mkDarwinService { }"
    ));
  }

  #[test]
  fn nix_calls_sees_through_parens_and_currying() {
    assert!(calls(
      "{ imports = [ ((foundation.lib.mkDarwinService) { } extra) ]; }"
    ));
  }

  #[test]
  fn nix_calls_ignores_comments_strings_and_uncalled_attrs() {
    assert!(!calls("# mkDarwinService\n{ }"));
    assert!(!calls("{ note = \"mkDarwinService\"; }"));
    assert!(!calls(
      "{ lib.mkDarwinService = 1; alias = foundation.lib.mkDarwinService; }"
    ));
  }

  const NIXOS_CALL: &str = "foundation.lib.mkNixosService { name = \"x\"; }\n";

  fn write_under(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
  }

  #[test]
  fn nix_call_present_finds_a_call_anywhere_in_source() {
    let dir = tempfile::tempdir().unwrap();
    write_under(dir.path(), "nix/modules/nixos-server.nix", NIXOS_CALL);
    assert!(matches!(
      nix_call_present(dir.path(), "mkNixosService", None),
      Verdict::Pass
    ));
  }

  #[test]
  fn nix_call_present_ignores_the_build_tree() {
    let dir = tempfile::tempdir().unwrap();
    write_under(dir.path(), "target/gen.nix", NIXOS_CALL);
    assert!(matches!(
      nix_call_present(dir.path(), "mkNixosService", None),
      Verdict::Fail { .. }
    ));
  }

  #[test]
  fn nix_call_present_ignores_a_result_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let linked = tempfile::tempdir().unwrap();
    write_under(linked.path(), "linked.nix", NIXOS_CALL);
    std::os::unix::fs::symlink(linked.path(), dir.path().join("result"))
      .unwrap();
    assert!(matches!(
      nix_call_present(dir.path(), "mkNixosService", None),
      Verdict::Fail { .. }
    ));
  }

  #[test]
  fn nix_call_present_fails_an_absent_target() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
      nix_call_present(dir.path(), "mkNixosService", Some("missing.nix")),
      Verdict::Fail { .. }
    ));
  }

  #[test]
  fn nix_call_present_fails_an_unparseable_file_by_name() {
    let dir = tempfile::tempdir().unwrap();
    write_under(dir.path(), "broken.nix", "{ = }\n");
    assert!(matches!(
      nix_call_present(dir.path(), "mkNixosService", None),
      Verdict::Fail { detail } if detail.contains("broken.nix")
    ));
  }
}
