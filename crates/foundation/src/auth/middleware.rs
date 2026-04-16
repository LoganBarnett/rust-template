//! Authentication middleware and helpers.

use axum::{
  extract::{Request, State},
  middleware::Next,
  response::{IntoResponse, Redirect, Response},
};
use openidconnect::core::CoreClient;
use std::sync::Arc;
use tower_sessions::Session;
use tracing::warn;

use super::types::AuthUser;

const KEY_USER: &str = "user";
const KEY_RETURN_TO: &str = "return_to";

/// Middleware that requires an authenticated session.
///
/// When OIDC is not configured (state is `None`), all requests pass through
/// immediately (every request is implicitly admin).  When OIDC is configured,
/// unauthenticated requests are redirected to `/auth/login`.
pub async fn require_auth(
  State(oidc_client): State<Option<Arc<CoreClient>>>,
  session: Session,
  req: Request,
  next: Next,
) -> Response {
  if oidc_client.is_none() {
    return next.run(req).await;
  }

  let user: Option<AuthUser> = session.get(KEY_USER).await.unwrap_or(None);

  if user.is_none() {
    let return_to = req.uri().to_string();
    if let Err(e) = session.insert(KEY_RETURN_TO, return_to).await {
      warn!("Failed to save return_to in session: {e}");
    }
    return Redirect::to("/auth/login").into_response();
  }

  next.run(req).await
}

/// Extract the current user from the session, if any.
///
/// Returns `None` for unauthenticated sessions rather than failing —
/// use `require_auth` on routes that must be protected.
pub async fn current_user(session: &Session) -> Option<AuthUser> {
  session.get(KEY_USER).await.unwrap_or(None)
}
