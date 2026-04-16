//! Single-page application fallback service.
//!
//! Serves static files from a directory with a fallback to `index.html` for
//! client-side routing, and sets `Cache-Control: no-store` so the browser
//! always fetches fresh content during development.

use axum::http::{header, HeaderValue};
use tower_http::{
  services::{ServeDir, ServeFile},
  set_header::SetResponseHeader,
};

/// Build a service that serves static files from `frontend_path` with an
/// `index.html` fallback and no-cache headers.
///
/// Use as `Router::fallback_service(spa_service(&path))`.
pub fn spa_service(
  frontend_path: &std::path::Path,
) -> SetResponseHeader<ServeDir<ServeFile>, HeaderValue> {
  SetResponseHeader::overriding(
    ServeDir::new(frontend_path)
      .fallback(ServeFile::new(frontend_path.join("index.html"))),
    header::CACHE_CONTROL,
    HeaderValue::from_static("no-store"),
  )
}
