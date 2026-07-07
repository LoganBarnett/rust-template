//! Compiled frontend assets, embedded into the server binary.
//!
//! The `build.rs` resolves the asset directory and exports it as
//! `RUST_TEMPLATE_FRONTEND_DIR`; `rust_embed` reads that at compile time.
//! In release builds the assets are baked into the binary, so a downloaded
//! release is self-contained; in debug builds they are read from disk, so
//! `elm make` output is picked up without recompiling the server.
//!
//! Install the SPA fallback with `Server::spa::<Frontend>()`.

/// Embedded compiled Elm frontend (`index.html`, `elm.js`, `style.css`).
#[derive(rust_embed::RustEmbed)]
#[folder = "$RUST_TEMPLATE_FRONTEND_DIR"]
pub struct Frontend;
