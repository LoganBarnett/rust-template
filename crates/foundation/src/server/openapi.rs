//! OpenAPI spec and Scalar UI route builder.

use aide::{openapi::OpenApi, scalar::Scalar};
use axum::{routing::get, Json, Router};
use std::sync::Arc;

/// Build routes that serve the OpenAPI JSON spec and the Scalar interactive
/// documentation UI.
pub fn openapi_routes(api: Arc<OpenApi>, title: &str) -> Router {
  let scalar_handler = Scalar::new("/api-docs/openapi.json")
    .with_title(title)
    .axum_handler();

  Router::new()
    .route(
      "/api-docs/openapi.json",
      get({
        let api = api.clone();
        move || async move { Json((*api).clone()) }
      }),
    )
    .route("/scalar", get(scalar_handler))
}
