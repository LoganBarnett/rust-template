//! Foundation-owned server runner: traits, config types, and `Server<S>`.
//!
//! The `Server` type assembles health checks, metrics, OpenAPI, auth,
//! SPA fallback, session management, and systemd integration into a
//! ready-to-listen Axum application.  Users interact with it through
//! builder methods — the internal assembly is identical to what was
//! previously scattered across each spawned project's `main.rs` and
//! `web_base.rs`.

use aide::{
  axum::{routing::get_with, ApiRouter},
  openapi::OpenApi,
  transform::TransformOperation,
};
use axum::{extract::FromRef, routing::get, serve, Router};
use openidconnect::core::CoreClient;
use prometheus::{IntCounter, Registry};
use std::{path::PathBuf, sync::Arc};
use thiserror::Error;
use tokio_listener::ListenerAddress;
use tower_http::trace::TraceLayer;
use tower_sessions::{cookie::SameSite, MemoryStore, SessionManagerLayer};
use tracing::{error, info};

use super::me;
pub use crate::app::CliApp;
use crate::auth::{self, OidcConfig, OidcDiscoveryError};
use crate::server::{
  health::HealthRegistry, metrics, openapi, shutdown, spa, systemd,
};

// ── traits ──────────────────────────────────────────────────────────────────

/// Extension for server apps that produce one or more server configs.
///
/// Single server (common case): return a `Vec` of one.  Multiple
/// servers (e.g. primary + admin): return a `Vec` of N and receive a
/// matching tuple of `Server` values in your entry point.
pub trait ServerApp: CliApp {
  fn server_run_configs(&self) -> Vec<ServerRunConfig>;
}

// ── config types ────────────────────────────────────────────────────────────

/// Everything the foundation needs to stand up one server instance.
pub struct ServerRunConfig {
  pub app_name: String,
  pub listen_address: ListenerAddress,
  pub frontend_path: Option<PathBuf>,
  pub base_url: String,
  pub oidc: Option<OidcConfig>,
}

// ── errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ServerError {
  #[error("OIDC provider discovery failed: {0}")]
  OidcDiscovery(#[from] OidcDiscoveryError),

  #[error("Failed to bind listener to {address}: {source}")]
  ListenerBind {
    address: String,
    #[source]
    source: std::io::Error,
  },

  #[error("Server runtime error: {0}")]
  Runtime(#[source] std::io::Error),
}

// ── BaseServerState ─────────────────────────────────────────────────────────

/// Shared infrastructure state created once and cloned (cheaply, via Arc)
/// into every `Server` instance.
#[derive(Clone)]
pub struct BaseServerState {
  pub health_registry: HealthRegistry,
  pub metrics_registry: Arc<Registry>,
  pub request_counter: IntCounter,
  pub oidc_client: Option<Arc<CoreClient>>,
  pub frontend_path: Option<PathBuf>,
}

impl BaseServerState {
  /// Initialise shared state: prometheus registry, OIDC discovery (if
  /// configured).  Uses the first server config's OIDC and frontend
  /// settings as the canonical source.
  pub async fn init(config: &ServerRunConfig) -> Result<Self, ServerError> {
    let registry = Registry::new();
    let request_counter =
      IntCounter::new("http_requests_total", "Total HTTP requests")
        .expect("counter creation must not fail");
    registry
      .register(Box::new(request_counter.clone()))
      .expect("counter registration must not fail");

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
      oidc_client,
      frontend_path: config.frontend_path.clone(),
    })
  }
}

// FromRef impls so foundation handlers can extract their state slice
// from BaseServerState directly (no custom AppState needed for simple
// servers).

impl FromRef<BaseServerState> for HealthRegistry {
  fn from_ref(state: &BaseServerState) -> Self {
    state.health_registry.clone()
  }
}

impl FromRef<BaseServerState> for Arc<Registry> {
  fn from_ref(state: &BaseServerState) -> Self {
    state.metrics_registry.clone()
  }
}

impl FromRef<BaseServerState> for Option<Arc<CoreClient>> {
  fn from_ref(state: &BaseServerState) -> Self {
    state.oidc_client.clone()
  }
}

// ── impl_server_state! macro ────────────────────────────────────────────────

/// Generate `FromRef` implementations that delegate to a
/// `BaseServerState` field.
///
/// ```ignore
/// impl_server_state!(AppState, base);
/// // Generates FromRef<AppState> for HealthRegistry, Arc<Registry>,
/// // Option<Arc<CoreClient>>.
/// ```
#[macro_export]
macro_rules! impl_server_state {
  ($state_ty:ty, $field:ident) => {
    impl ::axum::extract::FromRef<$state_ty>
      for $crate::server::health::HealthRegistry
    {
      fn from_ref(state: &$state_ty) -> Self {
        state.$field.health_registry.clone()
      }
    }

    impl ::axum::extract::FromRef<$state_ty>
      for ::std::sync::Arc<::prometheus::Registry>
    {
      fn from_ref(state: &$state_ty) -> Self {
        state.$field.metrics_registry.clone()
      }
    }

    impl ::axum::extract::FromRef<$state_ty>
      for ::std::option::Option<
        ::std::sync::Arc<::openidconnect::core::CoreClient>,
      >
    {
      fn from_ref(state: &$state_ty) -> Self {
        state.$field.oidc_client.clone()
      }
    }
  };
}

// ── Server<S> ───────────────────────────────────────────────────────────────

/// A foundation-managed server that assembles infrastructure routes,
/// auth, sessions, and user routes into a single Axum application.
pub struct Server<S = BaseServerState>
where
  S: Clone + Send + Sync + 'static,
{
  state: S,
  base: BaseServerState,
  router: ApiRouter<S>,
  config: ServerRunConfig,
}

impl Server<BaseServerState> {
  /// Create a server from shared base state and a run config.
  pub fn new(base: BaseServerState, config: ServerRunConfig) -> Self {
    Self {
      state: base.clone(),
      base,
      router: ApiRouter::new(),
      config,
    }
  }

  /// Access the shared base state (health registry, metrics, OIDC
  /// client).  Clone this to create additional servers on different
  /// ports/sockets.
  pub fn base_state(&self) -> &BaseServerState {
    &self.base
  }

  /// Transform to use custom application state.  The closure receives
  /// the `BaseServerState` and returns your custom state type.
  pub fn with_state<S2>(
    self,
    f: impl FnOnce(BaseServerState) -> S2,
  ) -> Server<S2>
  where
    S2: Clone + Send + Sync + 'static,
    HealthRegistry: FromRef<S2>,
    Arc<Registry>: FromRef<S2>,
    Option<Arc<CoreClient>>: FromRef<S2>,
  {
    let new_state = f(self.base.clone());
    Server {
      state: new_state,
      base: self.base,
      router: ApiRouter::new(),
      config: self.config,
    }
  }
}

impl<S> Server<S>
where
  S: Clone + Send + Sync + 'static,
  HealthRegistry: FromRef<S>,
  Arc<Registry>: FromRef<S>,
  Option<Arc<CoreClient>>: FromRef<S>,
{
  /// Add an OpenAPI-documented route.
  pub fn api_route(
    mut self,
    path: &str,
    method: aide::axum::routing::ApiMethodRouter<S>,
  ) -> Self {
    self.router = self.router.api_route(path, method);
    self
  }

  /// Merge an `ApiRouter` of user routes.
  pub fn merge(mut self, router: ApiRouter<S>) -> Self {
    self.router = self.router.merge(router);
    self
  }

  /// Start listening.  Blocks until graceful shutdown completes.
  pub async fn listen(self) -> Result<(), ServerError> {
    let listen_address = self.config.listen_address.to_string();
    let app = self.build_router(true);

    let parsed_address: ListenerAddress = listen_address
      .parse()
      .expect("round-tripping ListenerAddress through Display must succeed");

    let listener = tokio_listener::Listener::bind(
      &parsed_address,
      &tokio_listener::SystemOptions::default(),
      &tokio_listener::UserOptions::default(),
    )
    .await
    .map_err(|source| {
      error!("Failed to bind to {}: {}", listen_address, source);
      ServerError::ListenerBind {
        address: listen_address.clone(),
        source,
      }
    })?;

    info!("Server listening on {}", listen_address);

    systemd::notify_ready();
    systemd::spawn_watchdog();

    serve(listener, app.into_make_service())
      .with_graceful_shutdown(shutdown::shutdown_signal())
      .await
      .map_err(|e| {
        error!("Server error: {}", e);
        ServerError::Runtime(e)
      })?;

    info!("Server shut down");
    Ok(())
  }

  /// Build the complete `Router` without binding a listener.  Intended
  /// for integration tests where you drive the router with
  /// `tower::ServiceExt::oneshot`.  Uses `secure_cookies=false` so
  /// plain HTTP works in tests.
  pub fn into_test_router(self) -> Router {
    self.build_router(false)
  }

  /// Internal assembly — mirrors what was previously `create_app` +
  /// `base_router` in spawned projects.
  fn build_router(self, secure_cookies: bool) -> Router {
    aide::generate::extract_schemas(true);
    let app_name = self.config.app_name.clone();
    let state = self.state;
    let base = self.base;
    let mut api = OpenApi::default();

    // Base infrastructure routes (healthz, metrics) with OpenAPI docs.
    let infra_router = ApiRouter::new()
      .api_route(
        "/healthz",
        get_with(
          crate::server::health::healthz_handler,
          |op: TransformOperation| op.description("Health check."),
        ),
      )
      .api_route(
        "/metrics",
        get_with(metrics::metrics_handler, |op: TransformOperation| {
          op.description("Prometheus metrics in text/plain format.")
        }),
      );

    // Merge user routes with infra routes, then finalize OpenAPI.
    let api_router = infra_router
      .merge(self.router)
      .with_state(state)
      .finish_api_with(&mut api, |a| a.title(&app_name));

    let api = Arc::new(api);

    // Auth and /me routes — built via non-generic helpers so the
    // method routers resolve to Option<Arc<CoreClient>> rather than S.
    let auth_router = build_auth_routes(base.oidc_client.clone());
    let me_router = build_me_route(base.oidc_client.clone());

    // Assemble everything.
    let mut full = Router::new()
      .merge(api_router)
      .merge(auth_router)
      .merge(me_router)
      .merge(openapi::openapi_routes(api, &app_name));

    // SPA fallback when a frontend path is configured.
    if let Some(ref frontend_path) = base.frontend_path {
      full = full.fallback_service(spa::spa_service(frontend_path));
    }

    // Session layer.
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
      .with_secure(secure_cookies)
      .with_same_site(SameSite::Lax);

    full.layer(session_layer).layer(TraceLayer::new_for_http())
  }
}

// ── non-generic route builders ──────────────────────────────────────────────

/// Build auth routes with concrete `Option<Arc<CoreClient>>` state.
/// Separate function so the MethodRouter type parameters resolve to
/// the OIDC client state rather than the enclosing generic `S`.
fn build_auth_routes(oidc_client: Option<Arc<CoreClient>>) -> Router {
  Router::new()
    .route("/auth/login", get(auth::login_handler))
    .route("/auth/callback", get(auth::callback_handler))
    .route("/auth/logout", get(auth::logout_handler))
    .with_state(oidc_client)
}

/// Build the `/me` route with concrete state.
fn build_me_route(oidc_client: Option<Arc<CoreClient>>) -> Router {
  Router::new()
    .route("/me", get(me::me_handler))
    .with_state(oidc_client)
}
