//! The `provenance-schema` kind: `rust-template.json` matches the typed model
//! and lists every crate under `crates/`.

use super::{read_file, walk_files, FileRead, Verdict};
use crate::provenance::Provenance;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;

/// Check the spawn's `rust-template.json` against the typed provenance model.
/// The raw file is parsed here.  The context's copy cannot serve, because the
/// lenient loader degrades exactly these violations to an empty default.
pub(super) fn provenance_schema(dir: &Path) -> Verdict {
  match read_file(&dir.join("rust-template.json")) {
    FileRead::Found(text) => serde_json::from_str::<Provenance>(&text)
      .map_or_else(
        |error| Verdict::Fail {
          detail: format!(
            "rust-template.json does not match the schema: {error}"
          ),
        },
        |provenance| unlisted_members(dir, &provenance),
      ),
    FileRead::Missing => Verdict::Skip {
      reason: "rust-template.json not present".to_string(),
    },
    FileRead::Error(detail) => Verdict::Error { detail },
  }
}

/// Fail naming each crate under `crates/` that has no `workspaces` entry.
fn unlisted_members(dir: &Path, provenance: &Provenance) -> Verdict {
  let unlisted: Vec<String> = crate_member_names(dir)
    .into_iter()
    .filter(|name| !provenance.workspaces.contains_key(name))
    .collect();
  if unlisted.is_empty() {
    Verdict::Pass
  } else {
    Verdict::Fail {
      detail: format!(
        "crates missing a workspaces entry in rust-template.json: {}",
        unlisted.join(", ")
      ),
    }
  }
}

/// The directory name of every `crates/*/Cargo.toml`.
fn crate_member_names(dir: &Path) -> BTreeSet<String> {
  let crates = dir.join("crates");
  walk_files(&crates)
    .into_iter()
    .filter(|path| path.file_name() == Some(OsStr::new("Cargo.toml")))
    .filter_map(|manifest| {
      manifest
        .parent()
        .filter(|member| member.parent() == Some(crates.as_path()))
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
    })
    .collect()
}
