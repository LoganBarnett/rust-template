use axum::{
  body::Body,
  http::{Request, StatusCode},
};
use rust_template_web::web_base::{base_router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn test_healthz_endpoint() {
  let state = AppState::new();
  let app = base_router(state);

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
  let state = AppState::new();
  let app = base_router(state);

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
async fn test_openapi_endpoint() {
  let state = AppState::new();
  let app = base_router(state);

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

  assert!(
    body_str.contains("openapi"),
    "OpenAPI spec should contain 'openapi' field"
  );
  assert!(
    body_str.contains("/healthz"),
    "OpenAPI spec should document /healthz endpoint"
  );
  assert!(
    body_str.contains("/metrics"),
    "OpenAPI spec should document /metrics endpoint"
  );
}

#[tokio::test]
async fn test_swagger_ui_redirect() {
  let state = AppState::new();
  let app = base_router(state);

  let response = app
    .oneshot(
      Request::builder()
        .uri("/swagger-ui")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  // The swagger UI endpoint should redirect (or be accessible)
  assert!(
    response.status() == StatusCode::MOVED_PERMANENTLY
      || response.status() == StatusCode::PERMANENT_REDIRECT
      || response.status() == StatusCode::TEMPORARY_REDIRECT
      || response.status() == StatusCode::SEE_OTHER
      || response.status() == StatusCode::OK,
    "Swagger UI should be accessible at /swagger-ui, got status: {:?}",
    response.status()
  );
}
