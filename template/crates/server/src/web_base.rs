use aide::{
  axum::{routing::get_with, ApiRouter},
  openapi::OpenApi,
  transform::TransformOperation,
};
use axum::{extract::FromRef, routing::get, Json, Router};
use openidconnect::core::CoreClient;
use prometheus::{IntCounter, Registry};
use rust_template_foundation::{
  auth,
  server::{health::HealthRegistry, metrics, openapi, spa},
};
use schemars::JsonSchema;
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};
use thiserror::Error;
use tracing::info;

use crate::config::Config;

// ── AppState ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
  pub health_registry: HealthRegistry,
  pub metrics_registry: Arc<Registry>,
  pub request_counter: IntCounter,
  pub frontend_path: PathBuf,
  pub oidc_client: Option<Arc<CoreClient>>,
}

// FromRef impls so foundation handlers can extract their state slice.

impl FromRef<AppState> for HealthRegistry {
  fn from_ref(state: &AppState) -> Self {
    state.health_registry.clone()
  }
}

impl FromRef<AppState> for Arc<Registry> {
  fn from_ref(state: &AppState) -> Self {
    state.metrics_registry.clone()
  }
}

impl FromRef<AppState> for Option<Arc<CoreClient>> {
  fn from_ref(state: &AppState) -> Self {
    state.oidc_client.clone()
  }
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

impl From<auth::OidcDiscoveryError> for AppStateError {
  fn from(e: auth::OidcDiscoveryError) -> Self {
    match e {
      auth::OidcDiscoveryError::InvalidIssuer(s) => {
        AppStateError::InvalidIssuer(s)
      }
      auth::OidcDiscoveryError::Discovery(s) => AppStateError::OidcDiscovery(s),
      auth::OidcDiscoveryError::InvalidRedirectUri(s) => {
        AppStateError::InvalidRedirectUri(s)
      }
    }
  }
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
      Some(oidc) => Some(auth::discover_oidc(oidc, &config.base_url).await?),
      None => {
        info!("OIDC not configured — running unauthenticated");
        None
      }
    };

    Ok(Self {
      health_registry: HealthRegistry::default(),
      metrics_registry: Arc::new(registry),
      request_counter,
      frontend_path: config.frontend_path.clone(),
      oidc_client,
    })
  }
}

// ── base router ─────────────────────────────────────────────────────────────

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
      get_with(
        rust_template_foundation::server::health::healthz_handler,
        |op: TransformOperation| op.description("Health check."),
      ),
    )
    .api_route(
      "/metrics",
      get_with(metrics::metrics_handler, |op: TransformOperation| {
        op.description("Prometheus metrics in text/plain format.")
      }),
    )
    .with_state(state)
    .finish_api_with(&mut api, |a| a.title("rust-template"));

  let api = Arc::new(api);

  Router::new()
    .merge(app_router)
    .route("/me", get(me_handler).with_state(me_state))
    .merge(openapi::openapi_routes(api, "rust-template"))
    .fallback_service(spa::spa_service(&frontend_path))
}
