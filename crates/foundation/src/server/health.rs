//! Composable health-check registry.
//!
//! Components register lightweight [`HealthCheck`] implementations that read
//! cached state (e.g. `AtomicBool`).  The `/healthz` handler aggregates them
//! into a single response where the worst component status wins.
//!
//! # Example
//!
//! ```ignore
//! let registry = HealthRegistry::default();
//! registry.register("nats", NatsHealthCheck::new(client.clone())).await;
//!
//! // In your router:
//! .route("/healthz", get(healthz_handler))
//! .with_state(registry)
//! ```

use axum::{extract::State, http::StatusCode, Json};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── types ───────────────────────────────────────────────────────────────────

/// Health status of an individual component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentHealth {
  Healthy,
  Degraded(String),
  Unhealthy(String),
}

impl ComponentHealth {
  fn severity(&self) -> u8 {
    match self {
      ComponentHealth::Healthy => 0,
      ComponentHealth::Degraded(_) => 1,
      ComponentHealth::Unhealthy(_) => 2,
    }
  }
}

/// Implement this trait to provide health information for a component.
///
/// Implementations should read cached state rather than performing I/O,
/// keeping the endpoint fast.
pub trait HealthCheck: Send + Sync {
  fn check(&self) -> ComponentHealth;
}

// ── registry ────────────────────────────────────────────────────────────────

/// Thread-safe registry of named health checks.
#[derive(Clone, Default)]
pub struct HealthRegistry {
  checks: Arc<RwLock<Vec<(String, Arc<dyn HealthCheck>)>>>,
}

impl HealthRegistry {
  /// Register a named health check.
  pub async fn register(
    &self,
    name: impl Into<String>,
    check: impl HealthCheck + 'static,
  ) {
    self
      .checks
      .write()
      .await
      .push((name.into(), Arc::new(check)));
  }

  /// Evaluate all registered checks and produce an aggregate response.
  pub async fn evaluate(&self) -> HealthResponse {
    let checks = self.checks.read().await;

    if checks.is_empty() {
      return HealthResponse {
        status: "healthy".to_string(),
        components: BTreeMap::new(),
      };
    }

    let mut components = BTreeMap::new();
    let mut worst: u8 = 0;

    for (name, check) in checks.iter() {
      let health = check.check();
      worst = worst.max(health.severity());

      let detail = match &health {
        ComponentHealth::Healthy => ComponentDetail {
          status: "healthy".to_string(),
          message: None,
        },
        ComponentHealth::Degraded(msg) => ComponentDetail {
          status: "degraded".to_string(),
          message: Some(msg.clone()),
        },
        ComponentHealth::Unhealthy(msg) => ComponentDetail {
          status: "unhealthy".to_string(),
          message: Some(msg.clone()),
        },
      };

      components.insert(name.clone(), detail);
    }

    let status = match worst {
      0 => "healthy",
      1 => "degraded",
      _ => "unhealthy",
    }
    .to_string();

    HealthResponse { status, components }
  }
}

// ── response types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
pub struct ComponentDetail {
  pub status: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub message: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HealthResponse {
  pub status: String,
  #[serde(skip_serializing_if = "BTreeMap::is_empty")]
  pub components: BTreeMap<String, ComponentDetail>,
}

/// `GET /healthz` handler — takes `State<HealthRegistry>`.
pub async fn healthz_handler(
  State(registry): State<HealthRegistry>,
) -> (StatusCode, Json<HealthResponse>) {
  let response = registry.evaluate().await;
  let code = match response.status.as_str() {
    "healthy" | "degraded" => StatusCode::OK,
    _ => StatusCode::SERVICE_UNAVAILABLE,
  };
  (code, Json(response))
}
