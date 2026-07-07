//! Single-page application fallback service backed by embedded assets.
//!
//! Serves the compiled frontend baked into the binary via `rust_embed`,
//! falling back to `index.html` for any path that does not match an asset
//! (client-side routing).  Because the assets are embedded, a downloaded
//! release binary is self-contained — nothing has to ship alongside it and
//! no runtime path has to point at a separate asset directory.
//!
//! `rust_embed` reads the assets from disk in debug builds and embeds them
//! in release builds, so `cargo run` during frontend iteration still picks
//! up fresh `elm make` output without recompiling the server.

use axum::{
  http::{header, HeaderValue, StatusCode, Uri},
  response::{IntoResponse, Response},
  Router,
};
use rust_embed::RustEmbed;

/// Build a stateless router whose fallback serves the embedded frontend
/// assets `E`, with an `index.html` fallback for client-side routes.
///
/// Install it with `Server::spa::<E>()`, which hands the result to
/// `Router::fallback_service`.
pub fn spa_service<E: RustEmbed + 'static>() -> Router {
  Router::new().fallback(serve::<E>)
}

/// Resolve the request path against the embedded assets, serving
/// `index.html` for any miss so the SPA router owns unknown routes.
async fn serve<E: RustEmbed>(uri: Uri) -> Response {
  let path = uri.path().trim_start_matches('/');
  E::get(path).or_else(|| E::get("index.html")).map_or_else(
    || StatusCode::NOT_FOUND.into_response(),
    |file| {
      let content_type = HeaderValue::from_str(file.metadata.mimetype())
        .unwrap_or_else(|_| {
          HeaderValue::from_static("application/octet-stream")
        });
      (
        [
          (header::CONTENT_TYPE, content_type),
          // Match the previous serve-from-disk behaviour: never cache.
          // `index.html` cache-busts `elm.js` with the build hash, so
          // stale assets are not a concern and always-fresh is simplest.
          (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        file.data.into_owned(),
      )
        .into_response()
    },
  )
}
