//! Evaluate a spawn's service module.

use super::{nix_helper, Verdict};
use std::ffi::OsStr;
use std::path::Path;

/// Evaluate the spawn's module at `module` with the `nix/compliance` helper
/// named `helper`, which reads the platform's module system from the spawn's
/// own lock.
pub(super) fn module_evaluates(
  dir: &Path,
  template_dir: &Path,
  helper: &str,
  module: &str,
) -> Verdict {
  nix_helper(
    template_dir,
    helper,
    module,
    &[("spawn", dir.as_os_str()), ("module", OsStr::new(module))],
  )
}
