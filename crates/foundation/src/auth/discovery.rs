//! OIDC provider discovery.

use openidconnect::core::CoreClient;
use std::sync::Arc;
use thiserror::Error;
use tracing::info;

use super::types::OidcConfig;

#[derive(Debug, Error)]
pub enum OidcDiscoveryError {
  #[error("Invalid OIDC issuer URL: {0}")]
  InvalidIssuer(String),

  #[error("OIDC provider discovery failed: {0}")]
  Discovery(String),

  #[error("Invalid OIDC redirect URI: {0}")]
  InvalidRedirectUri(String),
}

/// Perform OIDC provider discovery and return a configured `CoreClient`.
///
/// `base_url` is the externally-reachable base of the application (e.g.
/// `https://example.com`); the redirect URI is constructed as
/// `{base_url}/auth/callback`.
pub async fn discover_oidc(
  config: &OidcConfig,
  base_url: &str,
) -> Result<Arc<CoreClient>, OidcDiscoveryError> {
  let issuer = openidconnect::IssuerUrl::new(config.issuer.clone())
    .map_err(|e| OidcDiscoveryError::InvalidIssuer(e.to_string()))?;

  let provider_metadata =
    openidconnect::core::CoreProviderMetadata::discover_async(
      issuer,
      openidconnect::reqwest::async_http_client,
    )
    .await
    .map_err(|e| OidcDiscoveryError::Discovery(format!("{e:?}")))?;

  info!(issuer = %config.issuer, "OIDC discovery complete");

  let redirect_url = openidconnect::RedirectUrl::new(format!(
    "{}/auth/callback",
    base_url.trim_end_matches('/')
  ))
  .map_err(|e| OidcDiscoveryError::InvalidRedirectUri(e.to_string()))?;

  // RequestBody sends client credentials in the POST body
  // (client_secret_post).  Some providers (e.g. Authelia) require this
  // instead of the HTTP Basic Auth default.
  let client = openidconnect::core::CoreClient::from_provider_metadata(
    provider_metadata,
    openidconnect::ClientId::new(config.client_id.clone()),
    Some(openidconnect::ClientSecret::new(config.client_secret.clone())),
  )
  .set_redirect_uri(redirect_url)
  .set_auth_type(openidconnect::AuthType::RequestBody);

  Ok(Arc::new(client))
}
