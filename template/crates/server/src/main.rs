//! rust-template-server - General-purpose long-running service
//!
//! This is the project's persistent process.  It is not specifically an
//! HTTP server — HTTP is present only as infrastructure for health
//! checks, metrics, and observability.  The server may primarily watch
//! files, communicate over a binary protocol, bridge between systems,
//! or serve any other role requiring a long-running process.  Build new
//! long-running functionality here rather than creating a separate
//! service; the logging, systemd integration, and graceful shutdown are
//! already wired up.
//!
//! # LLM Development Guidelines
//! When modifying this code:
//! - Keep configuration logic in config.rs
//! - Keep base web functionality (healthz, metrics, openapi) in web_base.rs
//! - Add new endpoints in separate modules, not in main.rs
//! - Maintain the staged configuration pattern (CliRaw -> ConfigFileRaw -> Config)
//! - Use semantic error types with thiserror - NO anyhow blindly wrapping errors
//! - Add context at each error site explaining WHAT failed and WHY
//! - Preserve graceful shutdown handling (SIGTERM/SIGINT)
//! - Keep logging structured and consistent
//! - Preserve systemd::notify_ready() and systemd::spawn_watchdog() after bind

mod logging;
mod systemd;

use rust_template_server::{auth, config, web_base};

use axum::{routing::get, serve, Router};
use clap::Parser;
use config::{CliRaw, Config, ConfigError};
use logging::init_logging;
use thiserror::Error;
use tokio::signal;
use tower_http::trace::TraceLayer;
use tower_sessions::{cookie::SameSite, MemoryStore, SessionManagerLayer};
use tracing::{error, info};
use web_base::{AppState, AppStateError};

#[derive(Debug, Error)]
enum ApplicationError {
  #[error("Failed to load configuration during startup: {0}")]
  ConfigurationLoad(#[from] ConfigError),

  #[error("Failed to initialize application state: {0}")]
  StateInit(#[from] AppStateError),

  #[error("Failed to bind listener to {address}: {source}")]
  ListenerBind {
    address: String,
    source: std::io::Error,
  },

  #[error("Server encountered a runtime error: {0}")]
  ServerRuntime(#[source] std::io::Error),
}

#[tokio::main]
async fn main() -> Result<(), ApplicationError> {
  let cli = CliRaw::parse();

  let config = Config::from_cli_and_file(cli).map_err(|e| {
    eprintln!("Configuration error: {}", e);
    ApplicationError::ConfigurationLoad(e)
  })?;

  init_logging(config.log_level, config.log_format);

  info!("Starting rust-template-server");
  info!("Configuration loaded successfully");
  info!("Binding to {}", config.listen_address);

  let state = AppState::init(&config).await.map_err(|e| {
    error!("Failed to initialize application state: {}", e);
    ApplicationError::StateInit(e)
  })?;

  let app = create_app(state);

  let listener = tokio_listener::Listener::bind(
    &config.listen_address,
    &tokio_listener::SystemOptions::default(),
    &tokio_listener::UserOptions::default(),
  )
  .await
  .map_err(|source| {
    error!("Failed to bind to {}: {}", config.listen_address, source);
    ApplicationError::ListenerBind {
      address: config.listen_address.to_string(),
      source,
    }
  })?;

  info!("Server listening on {}", config.listen_address);

  systemd::notify_ready();
  systemd::spawn_watchdog();

  serve(listener, app.into_make_service())
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(|e| {
      error!("Server error: {}", e);
      ApplicationError::ServerRuntime(e)
    })?;

  info!("Shutting down rust-template-server");
  Ok(())
}

fn create_app(state: AppState) -> Router {
  let session_store = MemoryStore::default();
  // SameSite::Lax is required: Strict suppresses the session cookie on the
  // cross-site redirect back from the OIDC provider.
  let session_layer = SessionManagerLayer::new(session_store)
    .with_secure(true)
    .with_same_site(SameSite::Lax);

  let auth_router = Router::new()
    .route("/auth/login", get(auth::login_handler))
    .route("/auth/callback", get(auth::callback_handler))
    .route("/auth/logout", get(auth::logout_handler))
    .with_state(state.clone());

  web_base::base_router(state)
    .merge(auth_router)
    .layer(session_layer)
    .layer(TraceLayer::new_for_http())
}

async fn shutdown_signal() {
  let ctrl_c = async {
    signal::ctrl_c()
      .await
      .expect("failed to install Ctrl+C handler");
  };

  #[cfg(unix)]
  let terminate = async {
    signal::unix::signal(signal::unix::SignalKind::terminate())
      .expect("failed to install signal handler")
      .recv()
      .await;
  };

  #[cfg(not(unix))]
  let terminate = std::future::pending::<()>();

  tokio::select! {
      _ = ctrl_c => {
          info!("Received Ctrl+C, shutting down gracefully");
      },
      _ = terminate => {
          info!("Received SIGTERM, shutting down gracefully");
      },
  }
}
