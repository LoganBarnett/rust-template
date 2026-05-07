//! JWT bearer-token authentication for service / automation callers.
//!
//! Companion to the session-based OIDC flow in [`super`].  Browser
//! users authenticate via authorization code → session cookie; service
//! clients authenticate by sending a signed JWT in
//! `Authorization: Bearer <token>` and having it validated against an
//! issuer's JWKS.
//!
//! Foundation supplies the building blocks; downstream projects compose
//! them into their own `AppState` and routers.  This is intentional —
//! the Claims type is application-specific, and forcing one shape into
//! `BaseServerState` would either lock callers into our claims shape or
//! require optional-state gymnastics.  Instead, depend on
//! [`build_decoder`] for JWKS-fetching plumbing and embed the returned
//! [`Decoder`] in your own state.
//!
//! # Wiring example
//!
//! ```ignore
//! use axum::{Router, routing::get, extract::FromRef, Json};
//! use rust_template_foundation::auth::jwt::{
//!   build_decoder, Claims, Decoder, JwtConfig, ServiceClaims,
//! };
//!
//! #[derive(Clone, FromRef)]
//! struct AppState {
//!   decoder: Decoder<ServiceClaims>,
//! }
//!
//! async fn protected(claims: Claims<ServiceClaims>) -> Json<String> {
//!   Json(claims.claims.sub)
//! }
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let config = JwtConfig {
//!   jwks_url: "https://issuer.example/.well-known/jwks.json".into(),
//!   issuer: "https://issuer.example/".into(),
//!   audiences: vec!["my-api".into()],
//!   algorithms: vec![jsonwebtoken::Algorithm::RS256],
//! };
//! let state = AppState { decoder: build_decoder(&config).await? };
//! let app = Router::new()
//!   .route("/protected", get(protected))
//!   .with_state(state);
//! # Ok(()) }
//! ```

use jsonwebtoken::{Algorithm, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

pub use axum_jwt_auth::{
  AuthError, BearerTokenExtractor, Claims, CookieTokenExtractor, Decoder,
  HeaderTokenExtractor, JwtDecoder, LocalDecoder, RemoteJwksDecoder,
  TokenExtractor,
};

// ── config ──────────────────────────────────────────────────────────────────

/// Parameters needed to validate JWTs against an issuer's JWKS.
///
/// `audiences` and `algorithms` are required (non-empty) so that
/// validation cannot silently accept tokens from a different audience
/// or signed with an unexpected algorithm.  An empty audience set
/// disables audience checking in `jsonwebtoken`, which is almost never
/// what you want for a service API.
#[derive(Debug, Clone)]
pub struct JwtConfig {
  pub jwks_url: String,
  pub issuer: String,
  pub audiences: Vec<String>,
  pub algorithms: Vec<Algorithm>,
}

impl JwtConfig {
  /// Build a `jsonwebtoken::Validation` with this config's algorithms,
  /// issuer, and audiences pre-set.
  ///
  /// An empty `algorithms` list falls back to RS256 (the OIDC default)
  /// rather than disabling algorithm checking.
  pub fn validation(&self) -> Validation {
    let primary = self.algorithms.first().copied().unwrap_or(Algorithm::RS256);
    let mut validation = Validation::new(primary);
    if !self.algorithms.is_empty() {
      validation.algorithms = self.algorithms.clone();
    }
    validation.set_issuer(&[&self.issuer]);
    if !self.audiences.is_empty() {
      validation.set_audience(&self.audiences);
    }
    validation
  }
}

// ── default claims shape ────────────────────────────────────────────────────

/// Default claims shape for service/automation tokens.
///
/// Carries the standard registered claims plus a `flatten`ed map for
/// provider-specific extras (scopes, roles, custom attributes, etc.).
/// Downstream projects with stricter typing can define their own
/// claims struct and parameterize [`Decoder`] / [`Claims`] over it
/// instead of reusing this one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceClaims {
  pub sub: String,
  pub iss: String,
  /// `aud` may be a string or an array of strings depending on the
  /// issuer.  Held as raw JSON so consumers can branch on shape.
  #[serde(default)]
  pub aud: serde_json::Value,
  pub exp: i64,
  #[serde(flatten)]
  pub extra: HashMap<String, serde_json::Value>,
}

// ── error ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum JwtError {
  /// The underlying decoder builder rejected the configuration (e.g.
  /// missing JWKS URL, missing algorithms, invalid validation rules).
  #[error("JWT decoder build failed: {0}")]
  DecoderBuild(String),

  /// Initial JWKS fetch failed.  The decoder needs at least one
  /// successful fetch before requests can be served, since otherwise
  /// every request would 401 with `KeyNotFound`.
  #[error("Initial JWKS fetch failed: {0}")]
  Initialize(String),
}

// ── public API ──────────────────────────────────────────────────────────────

/// Build a JWKS-backed decoder for the claims type `T`.
///
/// `T` is the application's claims struct — anything that implements
/// `DeserializeOwned`.  Use [`ServiceClaims`] when the registered
/// claims plus a flattened extras map are enough; define your own when
/// you need typed access to specific provider claims (`scope`,
/// `roles`, tenant identifiers, etc.):
///
/// ```ignore
/// #[derive(serde::Deserialize)]
/// struct MyClaims {
///   sub: String,
///   exp: i64,
///   #[serde(default)]
///   scope: String,
///   tenant_id: String,
/// }
///
/// let decoder = build_decoder::<MyClaims>(&config).await?;
/// // decoder: Decoder<MyClaims>; pair with Claims<MyClaims> in handlers.
/// ```
///
/// Performs the initial JWKS fetch synchronously (so a misconfigured
/// issuer surfaces as a startup error rather than a flood of 401s) and
/// spawns a background refresh task.  The returned [`Decoder`] is
/// ready to embed in your application state.
///
/// The background-refresh `CancellationToken` is intentionally
/// dropped: shutdown is driven by process exit, matching the rest of
/// this template.  Callers that need explicit refresh-task lifecycle
/// control should use [`RemoteJwksDecoder`] directly.
pub async fn build_decoder<T>(
  config: &JwtConfig,
) -> Result<Decoder<T>, JwtError>
where
  T: serde::de::DeserializeOwned + Send + Sync + 'static,
{
  let decoder = RemoteJwksDecoder::builder()
    .jwks_url(config.jwks_url.clone())
    .validation(config.validation())
    .build()
    .map_err(|e| JwtError::DecoderBuild(e.to_string()))?;

  decoder
    .initialize()
    .await
    .map_err(|e| JwtError::Initialize(e.to_string()))?;

  Ok(Arc::new(decoder))
}
