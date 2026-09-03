use std::path::Path;

/// Whether `program` resolves on the current PATH, as `command -v` would
/// answer it.
pub fn on_path(program: &str) -> bool {
  std::env::var_os("PATH").is_some_and(|path| {
    std::env::split_paths(&path)
      .any(|dir| Path::new(&dir).join(program).is_file())
  })
}
