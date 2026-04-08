use aide::{
  axum::{routing::get_with, ApiRouter},
  openapi::OpenApi,
  scalar::Scalar,
  transform::TransformOperation,
};
use axum::{
  http::{header, HeaderValue, StatusCode},
  response::{IntoResponse, Response},
  routing::get,
  Json, Router,
};
use openidconnect::core::CoreClient;
use prometheus::{Encoder, IntCounter, Registry, TextEncoder};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use thiserror::Error;
use tower::ServiceBuilder;
use tower_http::{
  services::{ServeDir, ServeFile},
  set_header::SetResponseHeaderLayer,
};
use tracing::info;

use crate::{auth, config::Config};

// ── AppState ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
  pub registry: Arc<Registry>,
  pub request_counter: IntCounter,
  pub frontend_path: PathBuf,
  pub oidc_client: Option<Arc<CoreClient>>,
}

#[derive(Debug, Error)]
pub enum AppStateError {
  #[error("Invalid OIDC issuer URL: {0}")]
  InvalidIssuer(String),

  #[error("OIDC provider discovery failed: {0}")]
  OidcDiscovery(String),

  #[error("Invalid OIDC redirect URI: {0}")]
  InvalidRedirectUri(String),
}

impl AppState {
  pub fn auth_enabled(&self) -> bool {
    self.oidc_client.is_some()
  }

  /// Construct `AppState` from a validated `Config`.
  ///
  /// Performs OIDC discovery when OIDC is configured (an async HTTP call).
  pub async fn init(config: &Config) -> Result<Self, AppStateError> {
    let registry = Registry::new();
    let request_counter =
      IntCounter::new("http_requests_total", "Total HTTP requests")
        .expect("Failed to create counter");
    registry
      .register(Box::new(request_counter.clone()))
      .expect("Failed to register counter");

    let oidc_client = match &config.oidc {
      Some(oidc) => {
        let issuer = openidconnect::IssuerUrl::new(oidc.issuer.clone())
          .map_err(|e| AppStateError::InvalidIssuer(e.to_string()))?;

        let provider_metadata =
          openidconnect::core::CoreProviderMetadata::discover_async(
            issuer,
            openidconnect::reqwest::async_http_client,
          )
          .await
          .map_err(|e| AppStateError::OidcDiscovery(e.to_string()))?;

        info!(issuer = %oidc.issuer, "OIDC discovery complete");

        let redirect_url = openidconnect::RedirectUrl::new(format!(
          "{}/auth/callback",
          config.base_url.trim_end_matches('/')
        ))
        .map_err(|e| AppStateError::InvalidRedirectUri(e.to_string()))?;

        // RequestBody sends client credentials in the POST body
        // (client_secret_post).  Some providers (e.g. Authelia) require this
        // instead of the HTTP Basic Auth default.
        let client = openidconnect::core::CoreClient::from_provider_metadata(
          provider_metadata,
          openidconnect::ClientId::new(oidc.client_id.clone()),
          Some(openidconnect::ClientSecret::new(oidc.client_secret.clone())),
        )
        .set_redirect_uri(redirect_url)
        .set_auth_type(openidconnect::AuthType::RequestBody);

        Some(Arc::new(client))
      }
      None => {
        info!("OIDC not configured — running unauthenticated");
        None
      }
    };

    Ok(Self {
      registry: Arc::new(registry),
      request_counter,
      frontend_path: config.frontend_path.clone(),
      oidc_client,
    })
  }
}

// ── base router ───────────────────────────────────────────────────────────────

#[derive(Serialize, JsonSchema)]
pub struct HealthResponse {
  status: String,
}

async fn healthz() -> Json<HealthResponse> {
  Json(HealthResponse {
    status: "healthy".to_string(),
  })
}

#[derive(Serialize, JsonSchema)]
pub struct MeResponse {
  name: String,
  auth_enabled: bool,
}

async fn me_handler(
  axum::extract::State(state): axum::extract::State<AppState>,
  session: tower_sessions::Session,
) -> Json<MeResponse> {
  if !state.auth_enabled() {
    return Json(MeResponse {
      name: "admin".to_string(),
      auth_enabled: false,
    });
  }

  let name = auth::current_user(&session)
    .await
    .map(|u| u.name)
    .unwrap_or_else(|| "anonymous".to_string());

  Json(MeResponse {
    name,
    auth_enabled: true,
  })
}

pub fn base_router(state: AppState) -> Router {
  aide::generate::extract_schemas(true);
  let frontend_path = state.frontend_path.clone();
  let me_state = state.clone();
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
    .route("/me", get(me_handler).with_state(me_state))
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
    .fallback_service(
      ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
          header::CACHE_CONTROL,
          HeaderValue::from_static("no-store"),
        ))
        .service(
          ServeDir::new(&frontend_path)
            .fallback(ServeFile::new(frontend_path.join("index.html"))),
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
