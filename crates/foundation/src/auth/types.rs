//! Shared types for OIDC authentication.

use serde::{Deserialize, Serialize};

/// Authenticated user stored in the session.
///
/// Populated from the OIDC ID token / userinfo `name` and `email` claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
  pub name: String,
  pub email: String,
}

/// Raw OIDC provider coordinates read from configuration.
///
/// Async discovery (the HTTP call to `.well-known/openid-configuration`)
/// happens separately in [`super::discovery::discover_oidc`].
#[derive(Debug, Clone)]
pub struct OidcConfig {
  pub issuer: String,
  pub client_id: String,
  pub client_secret: String,
}

/// Query parameters returned by the OIDC provider on the callback redirect.
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
  pub code: String,
  pub state: String,
}
