//! Black-box cases over the real binary.
//!
//! Each case gets its own git repository.  The binary rewrites the index of
//! whatever repository encloses its working directory, so a case run against
//! the checkout under test would rewrite the developer's own staged work.
//!
//! The formatter is the real `treefmt` driving the real `rustfmt`.  What the
//! binary guards is how those two read a project, so a stand-in would prove
//! nothing.

// Clippy's in-test exemption does not reach the free helper functions of an
// integration-test binary, so the panicking variants are permitted at file
// level here.  Panicking is the failure signal a test wants.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

/// A function no formatter would leave alone.
fn malformed(name: &str) -> String {
  format!("fn  {name}(  )  {{let _x=1;}}\n")
}

/// The same function as rustfmt writes it under the fixture's two-space
/// `rustfmt.toml`.  The indent is what shows the project's configuration was
/// found.
fn formatted(name: &str) -> String {
  format!("fn {name}() {{\n  let _x = 1;\n}}\n")
}

struct Repo {
  /// Holds both the repository and the case's scratch files.  Dropping it
  /// removes them.
  root: TempDir,
}

impl Repo {
  /// A repository with one commit holding a formatter configuration and a
  /// crate root that declares an out-of-line module.
  fn new() -> Self {
    let this = Self::unborn();
    this.git(&["add", "--all"]);
    this.git(&["commit", "--quiet", "--message", "initial"]);
    this
  }

  /// The same files with nothing committed, so there is no `HEAD` to compare
  /// the index against.
  fn unborn() -> Self {
    let this = Self::init();
    this.write(
      "treefmt.toml",
      "[formatter.rustfmt]\ncommand = \"rustfmt\"\n\
       options = [\"--edition\", \"2021\"]\nincludes = [\"*.rs\"]\n",
    );
    this.write("rustfmt.toml", "tab_spaces = 2\n");
    this.write("src/lib.rs", "mod child;\n");
    this.write("src/child.rs", "pub fn child() {}\n");
    this
  }

  /// An initialised repository with nothing in it, not even an index.  The
  /// identity and signing are pinned so the developer's own configuration
  /// cannot interfere with a commit.
  fn init() -> Self {
    let root = TempDir::new().expect("a scratch directory");
    std::fs::create_dir_all(root.path().join("repo")).expect("the repo dir");
    std::fs::create_dir_all(root.path().join("work")).expect("the work dir");
    let this = Self { root };
    this.git(&["init", "--quiet", "--initial-branch=main"]);
    this.git(&["config", "user.email", "case@example.invalid"]);
    this.git(&["config", "user.name", "Case"]);
    this.git(&["config", "commit.gpgsign", "false"]);
    this
  }

  fn dir(&self) -> PathBuf {
    self.root.path().join("repo")
  }

  fn work(&self) -> PathBuf {
    self.root.path().join("work")
  }

  /// A command that sees neither the developer's git configuration nor the
  /// environment of a git hook this test run may itself be under.  Either
  /// could point the command at a different repository or index.
  fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command
      .current_dir(self.dir())
      .env("HOME", self.work())
      .env("XDG_CONFIG_HOME", self.work())
      .env("GIT_CONFIG_NOSYSTEM", "1")
      .env_remove("GIT_DIR")
      .env_remove("GIT_INDEX_FILE")
      .env_remove("GIT_WORK_TREE");
    command
  }

  fn git(&self, args: &[&str]) -> String {
    let output = self.command("git").args(args).output().expect("git to run");
    assert!(
      output.status.success(),
      "git {args:?} failed: {}",
      String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
  }

  /// Run the binary in this repository, as a pre-commit hook would.
  fn format_staged(&self) -> Output {
    self
      .command(env!("CARGO_BIN_EXE_rust-template-format-staged-cli"))
      .output()
      .expect("the format-staged binary to run")
  }

  /// Run the binary and require that it succeeded.
  fn format_staged_ok(&self) {
    let output = self.format_staged();
    assert!(
      output.status.success(),
      "format-staged failed: {}",
      String::from_utf8_lossy(&output.stderr),
    );
  }

  fn path(&self, path: &str) -> PathBuf {
    self.dir().join(path)
  }

  fn write(&self, path: &str, contents: &str) {
    let full = self.path(path);
    full
      .parent()
      .map(std::fs::create_dir_all)
      .transpose()
      .expect("the parent directory");
    std::fs::write(full, contents).expect("to write the file");
  }

  fn append(&self, path: &str, contents: &str) {
    self.write(path, &format!("{}{contents}", self.read(path)));
  }

  fn read(&self, path: &str) -> String {
    std::fs::read_to_string(self.path(path)).expect("to read the file")
  }

  /// The staged content of `path`.
  fn staged(&self, path: &str) -> String {
    self.git(&["show", &format!(":{path}")])
  }

  /// The content of `path` in the last commit.
  fn committed(&self, path: &str) -> String {
    self.git(&["show", &format!("HEAD:{path}")])
  }

  /// The index entries with their modes and blob ids.
  fn index_entries(&self) -> String {
    self.git(&["ls-files", "--stage"])
  }

  fn index_bytes(&self) -> Vec<u8> {
    std::fs::read(self.path(".git/index")).expect("to read the index")
  }

  fn status(&self) -> String {
    self.git(&["status", "--porcelain"])
  }

  /// Make this repository commit through a hook that runs the binary, the
  /// way a project's `.cargo-husky/hooks/pre-commit` does.
  #[cfg(unix)]
  fn install_hook(&self) {
    use std::os::unix::fs::PermissionsExt;
    let hooks = self.work().join("hooks");
    std::fs::create_dir_all(&hooks).expect("the hooks dir");
    let hook = hooks.join("pre-commit");
    std::fs::write(
      &hook,
      format!(
        "#!/bin/sh\nexec \"{}\" \"$@\"\n",
        env!("CARGO_BIN_EXE_rust-template-format-staged-cli"),
      ),
    )
    .expect("to write the hook");
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
      .expect("to mark the hook executable");
    self.git(&["config", "core.hooksPath", &path_text(&hooks)]);
  }
}

fn path_text(path: &Path) -> String {
  path.to_str().expect("a UTF-8 path").to_string()
}

/// The help text is where a user learns why a file with unstaged changes
/// looks un-formatted after a commit.
#[test]
fn help_explains_the_unstaged_diff() {
  let output =
    Command::new(env!("CARGO_BIN_EXE_rust-template-format-staged-cli"))
      .arg("--help")
      .output()
      .expect("the format-staged binary to run");
  let help = String::from_utf8_lossy(&output.stdout);
  assert!(output.status.success(), "--help failed: {help}");
  assert!(
    help.contains("undoing of the formatting"),
    "--help does not explain the unstaged diff:\n{help}"
  );
}

#[test]
fn unstaged_hunk_stays_out_of_the_commit_and_untouched_on_disk() {
  let repo = Repo::new();
  repo.append("src/lib.rs", &malformed("staged_probe"));
  repo.git(&["add", "src/lib.rs"]);
  // Appending after the add leaves this function out of the index.  That is
  // the state `git add --patch` produces when one hunk is declined.
  repo.append("src/lib.rs", &malformed("unstaged_probe"));
  let working = repo.read("src/lib.rs");

  repo.format_staged_ok();
  repo.git(&["commit", "--quiet", "--message", "partial"]);

  // The crate root declares `mod child;`, so reaching this content at all
  // shows the out-of-line module resolved.
  assert_eq!(
    repo.committed("src/lib.rs"),
    format!("mod child;\n{}", formatted("staged_probe")),
  );
  assert_eq!(repo.read("src/lib.rs"), working);
  assert_eq!(repo.status(), " M src/lib.rs\n");
}

#[test]
fn fully_staged_file_is_formatted_in_the_working_tree_too() {
  let repo = Repo::new();
  repo.append("src/child.rs", &malformed("synced_probe"));
  repo.git(&["add", "src/child.rs"]);

  repo.format_staged_ok();

  let expected = format!("pub fn child() {{}}\n{}", formatted("synced_probe"));
  assert_eq!(repo.staged("src/child.rs"), expected);
  assert_eq!(repo.read("src/child.rs"), expected);
  assert_eq!(repo.status(), "M  src/child.rs\n");
}

#[test]
fn nested_out_of_line_module_resolves() {
  let repo = Repo::new();
  repo.write("src/lib.rs", "mod child;\nmod nested;\n");
  repo.write("src/nested/deep.rs", "pub fn deep() {}\n");
  repo.write(
    "src/nested.rs",
    &format!("mod deep;\n{}", malformed("nested_probe")),
  );
  repo.git(&["add", "--all"]);

  repo.format_staged_ok();

  assert_eq!(
    repo.staged("src/nested.rs"),
    format!("mod deep;\n{}", formatted("nested_probe")),
  );
}

#[test]
fn unborn_head_formats_the_first_commit() {
  let repo = Repo::unborn();
  repo.append("src/child.rs", &malformed("first_probe"));
  repo.git(&["add", "--all"]);

  repo.format_staged_ok();

  assert_eq!(
    repo.staged("src/child.rs"),
    format!("pub fn child() {{}}\n{}", formatted("first_probe")),
  );
}

#[test]
fn renamed_file_with_an_edit_is_formatted() {
  let repo = Repo::new();
  repo.git(&["mv", "src/child.rs", "src/kid.rs"]);
  repo.write("src/lib.rs", "mod kid;\n");
  repo.append("src/kid.rs", &malformed("renamed_probe"));
  repo.git(&["add", "--all"]);

  repo.format_staged_ok();

  assert_eq!(
    repo.staged("src/kid.rs"),
    format!("pub fn child() {{}}\n{}", formatted("renamed_probe")),
  );
}

#[test]
fn path_with_a_space_is_formatted() {
  let repo = Repo::new();
  repo.write("src/with space.rs", &malformed("spaced_probe"));
  repo.git(&["add", "--all"]);

  repo.format_staged_ok();

  assert_eq!(repo.staged("src/with space.rs"), formatted("spaced_probe"));
}

#[cfg(unix)]
#[test]
fn staged_symlink_is_left_alone() {
  let repo = Repo::new();
  std::os::unix::fs::symlink("src/lib.rs", repo.path("link.rs"))
    .expect("to create the symlink");
  repo.append("src/child.rs", &malformed("beside_link_probe"));
  repo.git(&["add", "--all"]);
  let link_before = repo.git(&["ls-files", "--stage", "link.rs"]);

  repo.format_staged_ok();

  assert!(link_before.starts_with("120000 "), "{link_before}");
  assert_eq!(repo.git(&["ls-files", "--stage", "link.rs"]), link_before);
  assert!(repo
    .staged("src/child.rs")
    .ends_with(&formatted("beside_link_probe")));
}

#[cfg(unix)]
#[test]
fn executable_bit_survives_formatting() {
  use std::os::unix::fs::PermissionsExt;
  let repo = Repo::new();
  repo.write("src/tool.rs", &malformed("executable_probe"));
  std::fs::set_permissions(
    repo.path("src/tool.rs"),
    std::fs::Permissions::from_mode(0o755),
  )
  .expect("to mark the file executable");
  repo.git(&["add", "--all"]);

  repo.format_staged_ok();

  assert!(repo
    .git(&["ls-files", "--stage", "src/tool.rs"])
    .starts_with("100755 "));
  assert_eq!(repo.staged("src/tool.rs"), formatted("executable_probe"));
  let mode = std::fs::metadata(repo.path("src/tool.rs"))
    .expect("the file's metadata")
    .permissions()
    .mode();
  assert_eq!(mode & 0o111, 0o111, "the working file lost its executable bit");
}

#[test]
fn formatter_failure_stops_the_run_and_leaves_the_index_alone() {
  let repo = Repo::new();
  repo.append("src/child.rs", "fn broken( {\n");
  repo.git(&["add", "src/child.rs"]);
  let before = repo.index_bytes();

  let output = repo.format_staged();

  assert!(!output.status.success(), "a syntax error must fail the run");
  assert_eq!(repo.index_bytes(), before);
  assert_eq!(repo.read("src/child.rs"), "pub fn child() {}\nfn broken( {\n");
}

#[test]
fn nothing_staged_leaves_the_index_file_untouched() {
  let repo = Repo::new();
  let before = repo.index_bytes();

  repo.format_staged_ok();

  assert_eq!(repo.index_bytes(), before);
}

#[test]
fn already_formatted_staged_content_leaves_the_index_file_untouched() {
  let repo = Repo::new();
  repo.append("src/child.rs", &formatted("tidy_probe"));
  repo.git(&["add", "src/child.rs"]);
  let before = repo.index_bytes();

  repo.format_staged_ok();

  assert_eq!(repo.index_bytes(), before);
}

/// `git write-tree` records a valid tree for the staged content in the index.
/// A rewrite that kept that record would let the commit reuse it and capture
/// the unformatted blob.
#[test]
fn recorded_tree_does_not_carry_unformatted_content_into_the_commit() {
  let repo = Repo::new();
  repo.append("src/child.rs", &malformed("cached_probe"));
  repo.git(&["add", "src/child.rs"]);
  repo.git(&["write-tree"]);

  repo.format_staged_ok();
  repo.git(&["commit", "--quiet", "--message", "cached"]);

  assert_eq!(
    repo.committed("src/child.rs"),
    format!("pub fn child() {{}}\n{}", formatted("cached_probe")),
  );
  assert_eq!(repo.status(), "");
}

/// `git commit --all` hands the hook a different index file than the
/// repository's own, named in `GIT_INDEX_FILE`.
#[cfg(unix)]
#[test]
fn commit_all_through_a_hook_commits_formatted_content() {
  let repo = Repo::new();
  repo.install_hook();
  repo.append("src/child.rs", &malformed("hook_probe"));

  repo.git(&[
    "commit",
    "--quiet",
    "--all",
    "--message",
    "through the hook",
  ]);

  let expected = format!("pub fn child() {{}}\n{}", formatted("hook_probe"));
  assert_eq!(repo.committed("src/child.rs"), expected);
  assert_eq!(repo.read("src/child.rs"), expected);
  assert_eq!(repo.status(), "");
}

#[test]
fn split_index_is_refused_before_anything_is_written() {
  let repo = Repo::new();
  repo.git(&["config", "core.splitIndex", "true"]);
  repo.append("src/child.rs", &malformed("split_probe"));
  repo.git(&["add", "src/child.rs"]);
  let staged = repo.index_entries();

  let output = repo.format_staged();

  assert!(!output.status.success(), "a split index must be refused");
  assert!(
    String::from_utf8_lossy(&output.stderr).contains("core.splitIndex"),
    "the refusal must name the setting",
  );
  assert_eq!(repo.index_entries(), staged);
}

#[test]
fn repository_with_nothing_added_is_a_quiet_success() {
  let repo = Repo::init();

  repo.format_staged_ok();

  assert!(!repo.path(".git/index").exists(), "no index should be created");
}

#[test]
fn sparse_index_is_refused_before_anything_is_written() {
  let repo = Repo::new();
  repo.write("outside/far.rs", "pub fn far() {}\n");
  repo.git(&["add", "--all"]);
  repo.git(&["commit", "--quiet", "--message", "outside the cone"]);
  // The directory left out of the cone collapses to one entry.  That single
  // entry is what makes the index sparse.
  repo.git(&["sparse-checkout", "set", "--cone", "--sparse-index", "src"]);
  repo.append("src/child.rs", &malformed("sparse_probe"));
  repo.git(&["add", "src/child.rs"]);
  let staged = repo.index_entries();

  let output = repo.format_staged();

  assert!(!output.status.success(), "a sparse index must be refused");
  assert!(
    String::from_utf8_lossy(&output.stderr).contains("sparse"),
    "the refusal must say why",
  );
  assert_eq!(repo.index_entries(), staged);
}
