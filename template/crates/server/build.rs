//! Resolve the directory holding the compiled frontend assets and export
//! it so the `rust_embed`-derived `Frontend` type can bake them into the
//! binary.
//!
//! In a Nix release build the packaging passes `RUST_TEMPLATE_FRONTEND_DIR`
//! pointing at the freshly compiled Elm derivation.  For a plain `cargo
//! build` / `cargo run` during development it defaults to the in-tree
//! `frontend/public` directory, so `elm make` output is embedded (release)
//! or read live (debug) with no extra configuration.

use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
  let frontend_dir = match std::env::var_os("RUST_TEMPLATE_FRONTEND_DIR") {
    Some(dir) => PathBuf::from(dir),
    None => PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?)
      .join("../../frontend/public"),
  };
  // Normalise away the `..` so the path baked into the debug-mode reader is
  // stable regardless of the working directory; fall back to the literal
  // path when the directory is not present yet (a later build re-runs this).
  let frontend_dir = frontend_dir.canonicalize().unwrap_or(frontend_dir);

  // `rust_embed` reads `$RUST_TEMPLATE_FRONTEND_DIR` at macro-expansion
  // time; re-export the resolved (and defaulted) path so the attribute
  // always sees a concrete directory.
  println!(
    "cargo:rustc-env=RUST_TEMPLATE_FRONTEND_DIR={}",
    frontend_dir.display(),
  );
  println!("cargo:rerun-if-env-changed=RUST_TEMPLATE_FRONTEND_DIR");
  println!("cargo:rerun-if-changed={}", frontend_dir.display());
  Ok(())
}
