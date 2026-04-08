use axum::{
  body::Body,
  http::{Request, StatusCode},
};
use openidconnect::{
  core::{
    CoreClient, CoreJwsSigningAlgorithm, CoreProviderMetadata,
    CoreResponseType, CoreSubjectIdentifierType,
  },
  AuthUrl, ClientId, EmptyAdditionalProviderMetadata, IssuerUrl,
  JsonWebKeySetUrl, ResponseTypes,
};
use prometheus::{IntCounter, Registry};
use rust_template_daemon::web_base::{base_router, AppState};
use std::{path::PathBuf, sync::Arc};
use tower::ServiceExt;

/// Builds a minimal `AppState` whose OIDC client is a stub.  Sufficient
/// for testing routes that never touch the OIDC flow.
fn stub_state(frontend_path: PathBuf) -> AppState {
  let registry = Registry::new();
  let request_counter =
    IntCounter::new("http_requests_total", "Total HTTP requests")
      .expect("counter creation");
  registry
    .register(Box::new(request_counter.clone()))
    .expect("counter registration");

  let issuer = IssuerUrl::new("https://stub.invalid".to_string()).unwrap();
  let metadata = CoreProviderMetadata::new(
    issuer,
    AuthUrl::new("https://stub.invalid/authorize".to_string()).unwrap(),
    JsonWebKeySetUrl::new("https://stub.invalid/jwks".to_string()).unwrap(),
    vec![ResponseTypes::new(vec![CoreResponseType::Code])],
    vec![CoreSubjectIdentifierType::Public],
    vec![CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256],
    EmptyAdditionalProviderMetadata {},
  );
  let oidc_client = CoreClient::from_provider_metadata(
    metadata,
    ClientId::new("stub-client".to_string()),
    None,
  );

  AppState {
    registry: Arc::new(registry),
    request_counter,
    frontend_path,
    oidc_client: Arc::new(oidc_client),
  }
}

// Tests that don't exercise the SPA fallback use a non-existent path since
// the registered API routes never touch the filesystem.
fn state_without_frontend() -> AppState {
  stub_state(PathBuf::from("/nonexistent"))
}

#[tokio::test]
async fn test_healthz_endpoint() {
  let app = base_router(state_without_frontend());

  let response = app
    .oneshot(
      Request::builder()
        .uri("/healthz")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  let body_str = String::from_utf8(body.to_vec()).unwrap();

  assert!(body_str.contains("healthy"));
}

#[tokio::test]
async fn test_metrics_endpoint() {
  let app = base_router(state_without_frontend());

  let response = app
    .oneshot(
      Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  let body_str = String::from_utf8(body.to_vec()).unwrap();

  assert!(
    body_str.contains("http_requests_total"),
    "Metrics should contain http_requests_total counter"
  );
}

#[tokio::test]
async fn test_openapi_json_endpoint() {
  let app = base_router(state_without_frontend());

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api-docs/openapi.json")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  let body_str = String::from_utf8(body.to_vec()).unwrap();

  assert!(body_str.contains("openapi"), "Response should be an OpenAPI spec");
  assert!(body_str.contains("/healthz"), "Spec should document /healthz");
  assert!(body_str.contains("/metrics"), "Spec should document /metrics");
}

#[tokio::test]
async fn test_scalar_ui_endpoint() {
  let app = base_router(state_without_frontend());

  let response = app
    .oneshot(
      Request::builder()
        .uri("/scalar")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();

  assert!(
    body.starts_with(b"<!doctype html>")
      || body.starts_with(b"<!DOCTYPE html>"),
    "Scalar endpoint should return HTML"
  );
}

#[tokio::test]
async fn test_spa_fallback_serves_index_html() {
  // Any path not matched by a registered route must return 200 with the SPA
  // index.html, not 404.  This covers direct navigation and page refresh at
  // client-side routes like /dashboard or /settings/profile.
  let frontend_dir = tempfile::tempdir().unwrap();
  std::fs::write(
    frontend_dir.path().join("index.html"),
    b"<!doctype html><title>rust-template</title>",
  )
  .unwrap();

  let app = base_router(stub_state(frontend_dir.path().to_path_buf()));

  for path in ["/some-page", "/nested/route", "/unknown"] {
    let response = app
      .clone()
      .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
      .await
      .unwrap();
    assert_eq!(
      response.status(),
      StatusCode::OK,
      "expected 200 for SPA path {path}"
    );
  }
}
