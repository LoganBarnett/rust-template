use axum::{
  http::StatusCode,
  response::{IntoResponse, Response},
  routing::get,
  Json, Router,
};
use prometheus::{Encoder, IntCounter, Registry, TextEncoder};
use serde_json::json;
use std::sync::Arc;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(Clone)]
pub struct AppState {
  pub registry: Arc<Registry>,
  pub request_counter: IntCounter,
}

impl AppState {
  pub fn new() -> Self {
    let registry = Registry::new();
    let request_counter =
      IntCounter::new("http_requests_total", "Total HTTP requests")
        .expect("Failed to create counter");

    registry
      .register(Box::new(request_counter.clone()))
      .expect("Failed to register counter");

    Self {
      registry: Arc::new(registry),
      request_counter,
    }
  }
}

#[derive(OpenApi)]
#[openapi(
    paths(healthz, metrics_endpoint),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "metrics", description = "Metrics endpoints")
    )
)]
pub struct ApiDoc;

pub fn base_router(state: AppState) -> Router {
  let openapi = ApiDoc::openapi();

  Router::new()
    .route("/healthz", get(healthz))
    .route("/metrics", get(metrics_endpoint))
    .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", openapi))
    .with_state(state)
}

#[utoipa::path(
    get,
    path = "/healthz",
    tag = "health",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse)
    )
)]
async fn healthz() -> Json<HealthResponse> {
  Json(HealthResponse {
    status: "healthy".to_string(),
  })
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct HealthResponse {
  status: String,
}

#[utoipa::path(
    get,
    path = "/metrics",
    tag = "metrics",
    responses(
        (status = 200, description = "Prometheus metrics", content_type = "text/plain")
    )
)]
async fn metrics_endpoint(
  axum::extract::State(state): axum::extract::State<AppState>,
) -> Response {
  let encoder = TextEncoder::new();
  let metric_families = state.registry.gather();
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
