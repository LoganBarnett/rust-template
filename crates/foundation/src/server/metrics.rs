//! Prometheus metrics endpoint.
//!
//! The application registers its own metrics into a `prometheus::Registry`;
//! this handler encodes whatever is in the registry at request time.

use axum::{
  extract::State,
  http::StatusCode,
  response::{IntoResponse, Response},
  Json,
};
use prometheus::{Encoder, Registry, TextEncoder};
use serde_json::json;
use std::sync::Arc;

/// `GET /metrics` handler — takes `State<Arc<Registry>>`.
pub async fn metrics_handler(
  State(registry): State<Arc<Registry>>,
) -> Response {
  let encoder = TextEncoder::new();
  let metric_families = registry.gather();
  let mut buffer = Vec::new();

  match encoder.encode(&metric_families, &mut buffer) {
    Ok(_) => {
      (StatusCode::OK, [("content-type", encoder.format_type())], buffer)
        .into_response()
    }
    Err(e) => (
      StatusCode::INTERNAL_SERVER_ERROR,
      Json(json!({
          "error": format!("Failed to encode metrics: {}", e)
      })),
    )
      .into_response(),
  }
}
