//! Integration tests for the foundation server infrastructure.
//!
//! Uses `Server::into_test_router()` + `tower::ServiceExt::oneshot`
//! to drive requests without binding a real listener.

#![cfg(feature = "auth")]

mod helpers;

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

// ── helpers ─────────────────────────────────────────────────────────────────

fn base_state_no_auth() -> BaseServerState {
  let registry = prometheus::Registry::new();
  let request_counter =
    prometheus::IntCounter::new("http_requests_total", "Total HTTP requests")
      .expect("counter creation");
  registry
    .register(Box::new(request_counter.clone()))
    .expect("counter registration");

  BaseServerState {
    health_registry: HealthRegistry::default(),
    metrics_registry: Arc::new(registry),
    request_counter,
    oidc_client: None,
    frontend_path: None,
  }
}

fn base_state_with_auth() -> BaseServerState {
  let mut state = base_state_no_auth();
  state.oidc_client = Some(helpers::stub_oidc_client());
  state
}

fn test_config(app_name: &str) -> ServerRunConfig {
  ServerRunConfig {
    app_name: app_name.to_string(),
    listen_address: "127.0.0.1:0".parse().unwrap(),
    frontend_path: None,
    base_url: "https://example.com".to_string(),
    oidc: None,
  }
}

fn server_no_auth() -> Server {
  Server::new(base_state_no_auth(), test_config("test-app"))
}

fn server_with_auth() -> Server {
  Server::new(base_state_with_auth(), test_config("test-app"))
}

async fn body_string(body: Body) -> String {
  let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
  String::from_utf8(bytes.to_vec()).unwrap()
}

// ── healthz ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn healthz_returns_ok() {
  let app = server_no_auth().into_test_router();
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
  let body = body_string(resp.into_body()).await;
  assert!(body.contains("healthy"));
}

// ── metrics ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn metrics_returns_prometheus_text() {
  let app = server_no_auth().into_test_router();
  let resp = app
    .oneshot(
      Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(resp.status(), StatusCode::OK);
  let body = body_string(resp.into_body()).await;
  assert!(
    body.contains("http_requests_total"),
    "metrics should contain http_requests_total counter"
  );
}

// ── OpenAPI ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn openapi_documents_healthz_and_metrics() {
  let app = server_no_auth().into_test_router();
  let resp = app
    .oneshot(
      Request::builder()
        .uri("/api-docs/openapi.json")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(resp.status(), StatusCode::OK);
  let body = body_string(resp.into_body()).await;
  assert!(body.contains("openapi"), "should be an OpenAPI spec");
  assert!(body.contains("/healthz"), "spec should document /healthz");
  assert!(body.contains("/metrics"), "spec should document /metrics");
}

// ── Scalar UI ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn scalar_ui_serves_html() {
  let app = server_no_auth().into_test_router();
  let resp = app
    .oneshot(
      Request::builder()
        .uri("/scalar")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(resp.status(), StatusCode::OK);
  let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
    .await
    .unwrap();
  assert!(
    body.starts_with(b"<!doctype html>")
      || body.starts_with(b"<!DOCTYPE html>"),
    "Scalar endpoint should return HTML"
  );
}

// ── /me endpoint ────────────────────────────────────────────────────────────

#[tokio::test]
async fn me_returns_admin_when_no_oidc() {
  let app = server_no_auth().into_test_router();
  let resp = app
    .oneshot(Request::builder().uri("/me").body(Body::empty()).unwrap())
    .await
    .unwrap();

  assert_eq!(resp.status(), StatusCode::OK);
  let body = body_string(resp.into_body()).await;
  let json: serde_json::Value = serde_json::from_str(&body).unwrap();
  assert_eq!(json["name"], "admin");
  assert_eq!(json["auth_enabled"], false);
}

#[tokio::test]
async fn me_returns_anonymous_with_oidc_no_session() {
  let app = server_with_auth().into_test_router();
  let resp = app
    .oneshot(Request::builder().uri("/me").body(Body::empty()).unwrap())
    .await
    .unwrap();

  assert_eq!(resp.status(), StatusCode::OK);
  let body = body_string(resp.into_body()).await;
  let json: serde_json::Value = serde_json::from_str(&body).unwrap();
  assert_eq!(json["name"], "anonymous");
  assert_eq!(json["auth_enabled"], true);
}

// ── auth routes ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn auth_routes_return_404_without_oidc() {
  let app = server_no_auth().into_test_router();

  for path in ["/auth/login", "/auth/logout"] {
    let resp = app
      .clone()
      .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
      .await
      .unwrap();
    assert_eq!(
      resp.status(),
      StatusCode::NOT_FOUND,
      "expected 404 for {path} without OIDC"
    );
  }

  let resp = app
    .oneshot(
      Request::builder()
        .uri("/auth/callback?code=x&state=y")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn auth_login_redirects_with_oidc() {
  let app = server_with_auth().into_test_router();
  let resp = app
    .oneshot(
      Request::builder()
        .uri("/auth/login")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(resp.status(), StatusCode::SEE_OTHER);
  let location = resp
    .headers()
    .get("location")
    .expect("redirect should have Location header")
    .to_str()
    .unwrap();
  assert!(
    location.contains("stub.invalid"),
    "redirect should point at the stub OIDC provider"
  );
}

// ── SPA fallback ────────────────────────────────────────────────────────────

#[tokio::test]
async fn spa_fallback_serves_index_html() {
  let frontend_dir = tempfile::tempdir().unwrap();
  std::fs::write(
    frontend_dir.path().join("index.html"),
    b"<!doctype html><title>test</title>",
  )
  .unwrap();

  let mut base = base_state_no_auth();
  base.frontend_path = Some(frontend_dir.path().to_path_buf());

  let mut config = test_config("test-spa");
  config.frontend_path = Some(frontend_dir.path().to_path_buf());

  let app = Server::new(base, config).into_test_router();

  for path in ["/some-page", "/nested/route", "/unknown"] {
    let resp = app
      .clone()
      .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
      .await
      .unwrap();
    assert_eq!(
      resp.status(),
      StatusCode::OK,
      "expected 200 for SPA path {path}"
    );
  }
}

#[tokio::test]
async fn no_spa_when_frontend_path_none() {
  let app = server_no_auth().into_test_router();
  let resp = app
    .oneshot(
      Request::builder()
        .uri("/nonexistent-path")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── with_state transform ────────────────────────────────────────────────────

#[tokio::test]
async fn with_state_transforms_server() {
  #[derive(Clone)]
  struct CustomState {
    base: BaseServerState,
    #[allow(dead_code)]
    custom_field: String,
  }

  rust_template_foundation::impl_server_state!(CustomState, base);

  let server = server_no_auth().with_state(|base| CustomState {
    base,
    custom_field: "test".to_string(),
  });

  let app = server.into_test_router();
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

// ── custom routes in OpenAPI ────────────────────────────────────────────────

#[tokio::test]
async fn custom_routes_appear_in_openapi() {
  use aide::axum::routing::get_with;
  use aide::transform::TransformOperation;
  use axum::Json;

  async fn custom_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({"hello": "world"}))
  }

  let server = server_no_auth().api_route(
    "/api/custom",
    get_with(custom_handler, |op: TransformOperation| {
      op.description("Custom endpoint.")
    }),
  );

  let app = server.into_test_router();

  // Verify the custom route works.
  let resp = app
    .clone()
    .oneshot(
      Request::builder()
        .uri("/api/custom")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::OK);

  // Verify it appears in OpenAPI spec.
  let resp = app
    .oneshot(
      Request::builder()
        .uri("/api-docs/openapi.json")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  let body = body_string(resp.into_body()).await;
  assert!(
    body.contains("/api/custom"),
    "OpenAPI spec should document /api/custom"
  );
}
