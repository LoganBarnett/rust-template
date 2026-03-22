use aide::{
  axum::{routing::get_with, ApiRouter},
  openapi::OpenApi,
  scalar::Scalar,
  transform::TransformOperation,
};
use axum::{
  http::StatusCode,
  response::{IntoResponse, Response},
  routing::get,
  Json, Router,
};
use prometheus::{Encoder, IntCounter, Registry, TextEncoder};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

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

#[derive(Serialize, JsonSchema)]
pub struct HealthResponse {
  status: String,
}

async fn healthz() -> Json<HealthResponse> {
  Json(HealthResponse {
    status: "healthy".to_string(),
  })
}

pub fn base_router(state: AppState) -> Router {
  aide::generate::extract_schemas(true);
  let mut api = OpenApi::default();

  let app_router = ApiRouter::new()
    .api_route(
      "/healthz",
      get_with(healthz, |op: TransformOperation| {
        op.description("Health check.")
      }),
    )
    .api_route(
      "/metrics",
      get_with(metrics_endpoint, |op: TransformOperation| {
        op.description("Prometheus metrics in text/plain format.")
      }),
    )
    .with_state(state)
    .finish_api_with(&mut api, |a| a.title("rust-template"));

  let api = Arc::new(api);

  Router::new()
    .merge(app_router)
    .route(
      "/api-docs/openapi.json",
      get({
        let api = api.clone();
        move || async move { Json((*api).clone()) }
      }),
    )
    .route(
      "/scalar",
      get(
        Scalar::new("/api-docs/openapi.json")
          .with_title("rust-template")
          .axum_handler(),
      ),
    )
}

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
