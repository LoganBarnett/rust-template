use aide::axum::routing::get_with;
use aide::transform::TransformOperation;
use rust_template_foundation::main as foundation_main;
use rust_template_foundation::{Server, ServerError};
use schemars::JsonSchema;
use serde::Serialize;
use std::process::ExitCode;

mod config;
use config::Config;

#[derive(Serialize, JsonSchema)]
struct HelloResponse {
  message: String,
}

async fn hello_handler() -> axum::Json<HelloResponse> {
  axum::Json(HelloResponse {
    message: "Hello from example server!".to_string(),
  })
}

#[foundation_main]
pub async fn main(
  _config: Config,
  server: Server,
) -> Result<ExitCode, ServerError> {
  let server = server.api_route(
    "/api/hello",
    get_with(hello_handler, |op: TransformOperation| {
      op.description("Say hello.")
    }),
  );
  server.listen().await?;
  Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
  use super::*;
  use axum::{
    body::Body,
    http::{Request, StatusCode},
  };
  use rust_template_foundation::server::health::HealthRegistry;
  use rust_template_foundation::server::runner::{
    BaseServerState, Server, ServerRunConfig,
  };
  use std::sync::Arc;
  use tower::ServiceExt;

  fn test_server() -> Server {
    let registry = prometheus::Registry::new();
    let request_counter =
      prometheus::IntCounter::new("http_requests_total", "requests").unwrap();
    registry
      .register(Box::new(request_counter.clone()))
      .unwrap();

    let base = BaseServerState {
      health_registry: HealthRegistry::default(),
      metrics_registry: Arc::new(registry),
      request_counter,
      oidc_client: None,
      frontend_path: None,
    };

    let config = ServerRunConfig {
      app_name: "example-server".to_string(),
      listen_address: "127.0.0.1:0".parse().unwrap(),
      frontend_path: None,
      base_url: "https://example.com".to_string(),
      oidc: None,
    };

    Server::new(base, config)
  }

  #[tokio::test]
  async fn hello_endpoint_works() {
    let server = test_server().api_route(
      "/api/hello",
      get_with(hello_handler, |op: TransformOperation| {
        op.description("Say hello.")
      }),
    );

    let app = server.into_test_router();
    let resp = app
      .oneshot(
        Request::builder()
          .uri("/api/hello")
          .body(Body::empty())
          .unwrap(),
      )
      .await
      .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
      .await
      .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["message"], "Hello from example server!");
  }

  #[tokio::test]
  async fn healthz_works() {
    let app = test_server().into_test_router();
    let resp = app
      .oneshot(
        Request::builder()
          .uri("/healthz")
          .body(Body::empty())
          .unwrap(),
      )
      .await
      .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
  }
}
