//! `/me` endpoint — returns the current user or a default identity.
//!
//! When OIDC is not configured, every request is implicitly admin.  When
//! OIDC is configured but the session has no authenticated user, the
//! response shows "anonymous".

use axum::{extract::State, Json};
use openidconnect::core::CoreClient;
use schemars::JsonSchema;
use serde::Serialize;
use std::sync::Arc;
use tower_sessions::Session;

use crate::auth;

/// JSON body returned by `GET /me`.
#[derive(Serialize, JsonSchema)]
pub struct MeResponse {
  name: String,
  auth_enabled: bool,
}

/// `GET /me` handler — takes `State<Option<Arc<CoreClient>>>` via `FromRef`.
pub async fn me_handler(
  State(oidc_client): State<Option<Arc<CoreClient>>>,
  session: Session,
) -> Json<MeResponse> {
  if oidc_client.is_none() {
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
