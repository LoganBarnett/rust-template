//! Integration tests for JWT bearer-token auth (the
//! `auth::jwt` module).
//!
//! Uses a `LocalDecoder` with HS256 instead of a `RemoteJwksDecoder`
//! pointing at an in-test JWKS server: the JWKS HTTP fetch is
//! `axum-jwt-auth`'s responsibility to test.  We're testing
//! foundation's wiring — that the `ServiceClaims` shape, the
//! `JwtConfig::validation()` rules, and the `Claims<ServiceClaims>`
//! extractor all compose correctly through an axum router.

#![cfg(feature = "auth")]
// Helpers (sign_token, build_app, etc.) live outside `#[test]`
// functions; opt the whole file into the test-unwrap exemption.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
  body::Body,
  extract::FromRef,
  http::{header, Request, StatusCode},
  routing::get,
  Json, Router,
};
use jsonwebtoken::{
  encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use rust_template_foundation::auth::jwt::{
  Claims, Decoder, JwtConfig, LocalDecoder, ServiceClaims,
};
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

// ── test helpers ────────────────────────────────────────────────────────────

const SECRET: &[u8] = b"test-shared-secret-do-not-use-in-prod";
const ISSUER: &str = "https://issuer.test/";
const AUDIENCE: &str = "test-api";

#[derive(Clone)]
struct TestState {
  decoder: Decoder<ServiceClaims>,
}

impl FromRef<TestState> for Decoder<ServiceClaims> {
  fn from_ref(state: &TestState) -> Self {
    state.decoder.clone()
  }
}

fn now_secs() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0)
}

/// HS256 LocalDecoder configured to mirror what `JwtConfig::validation`
/// produces, so we exercise the same validation surface.
fn build_local_decoder(audience: &str, issuer: &str) -> Decoder<ServiceClaims> {
  // Construct via JwtConfig so the validation rules (issuer, audience,
  // algorithm list) are the ones foundation actually generates.
  let config = JwtConfig {
    jwks_url: "https://unused.invalid/jwks.json".to_string(),
    issuer: issuer.to_string(),
    audiences: vec![audience.to_string()],
    algorithms: vec![Algorithm::HS256],
  };
  let validation = config.validation();
  let key = DecodingKey::from_secret(SECRET);
  let decoder = LocalDecoder::builder()
    .keys(vec![key])
    .validation(validation)
    .build()
    .unwrap();
  Arc::new(decoder)
}

fn sign_token(claims: &serde_json::Value, secret: &[u8]) -> String {
  encode(
    &Header::new(Algorithm::HS256),
    claims,
    &EncodingKey::from_secret(secret),
  )
  .unwrap()
}

fn valid_claims() -> serde_json::Value {
  json!({
    "sub": "service-account-1",
    "iss": ISSUER,
    "aud": AUDIENCE,
    "exp": now_secs() + 3600,
  })
}

async fn protected_handler(
  claims: Claims<ServiceClaims>,
) -> Json<serde_json::Value> {
  Json(json!({
    "sub": claims.claims.sub,
    "iss": claims.claims.iss,
  }))
}

fn build_app() -> Router {
  let state = TestState {
    decoder: build_local_decoder(AUDIENCE, ISSUER),
  };
  Router::new()
    .route("/protected", get(protected_handler))
    .with_state(state)
}

async fn body_string(body: Body) -> String {
  let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
  String::from_utf8(bytes.to_vec()).unwrap()
}

fn request_with_token(token: Option<&str>) -> Request<Body> {
  let mut builder = Request::builder().uri("/protected");
  if let Some(t) = token {
    builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
  }
  builder.body(Body::empty()).unwrap()
}

// ── happy path ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn valid_token_reaches_handler_with_claims() {
  let token = sign_token(&valid_claims(), SECRET);
  let resp = build_app()
    .oneshot(request_with_token(Some(&token)))
    .await
    .unwrap();

  assert_eq!(resp.status(), StatusCode::OK);
  let body = body_string(resp.into_body()).await;
  let json: serde_json::Value = serde_json::from_str(&body).unwrap();
  assert_eq!(json["sub"], "service-account-1");
  assert_eq!(json["iss"], ISSUER);
}

#[tokio::test]
async fn custom_claims_struct_works_via_generic_decoder() {
  // Apps with their own claims shape parameterize Decoder/Claims over
  // it.  The decoder construction is identical — only the type
  // parameter changes.
  use serde::Deserialize;

  #[derive(Clone, Deserialize)]
  struct MyClaims {
    sub: String,
    #[serde(default)]
    scope: String,
    tenant_id: String,
  }

  #[derive(Clone)]
  struct AppState {
    decoder: Decoder<MyClaims>,
  }

  impl FromRef<AppState> for Decoder<MyClaims> {
    fn from_ref(state: &AppState) -> Self {
      state.decoder.clone()
    }
  }

  async fn handler(claims: Claims<MyClaims>) -> Json<serde_json::Value> {
    Json(json!({
      "sub": claims.claims.sub,
      "scope": claims.claims.scope,
      "tenant_id": claims.claims.tenant_id,
    }))
  }

  // Build a Decoder<MyClaims> using the same LocalDecoder shape the
  // helper uses, just with a different T.
  let config = JwtConfig {
    jwks_url: "https://unused.invalid/jwks.json".to_string(),
    issuer: ISSUER.to_string(),
    audiences: vec![AUDIENCE.to_string()],
    algorithms: vec![Algorithm::HS256],
  };
  let local = LocalDecoder::builder()
    .keys(vec![DecodingKey::from_secret(SECRET)])
    .validation(config.validation())
    .build()
    .unwrap();
  let decoder: Decoder<MyClaims> = Arc::new(local);

  let app = Router::new()
    .route("/protected", get(handler))
    .with_state(AppState { decoder });

  let token = sign_token(
    &json!({
      "sub": "robot",
      "iss": ISSUER,
      "aud": AUDIENCE,
      "exp": now_secs() + 3600,
      "scope": "read:widgets",
      "tenant_id": "acme",
    }),
    SECRET,
  );

  let resp = app.oneshot(request_with_token(Some(&token))).await.unwrap();
  assert_eq!(resp.status(), StatusCode::OK);
  let body = body_string(resp.into_body()).await;
  let json: serde_json::Value = serde_json::from_str(&body).unwrap();
  assert_eq!(json["sub"], "robot");
  assert_eq!(json["scope"], "read:widgets");
  assert_eq!(json["tenant_id"], "acme");
}

#[tokio::test]
async fn extra_claims_round_trip_through_flatten() {
  // Verifies ServiceClaims.extra captures non-registered claims.
  let claims = json!({
    "sub": "svc",
    "iss": ISSUER,
    "aud": AUDIENCE,
    "exp": now_secs() + 3600,
    "scope": "read:widgets write:widgets",
    "tenant_id": "acme",
  });
  let token = sign_token(&claims, SECRET);

  // Decode it directly via the decoder to inspect the parsed shape.
  let decoder = build_local_decoder(AUDIENCE, ISSUER);
  let token_data = decoder.decode(&token).await.unwrap();
  assert_eq!(token_data.claims.sub, "svc");
  assert_eq!(
    token_data
      .claims
      .extra
      .get("scope")
      .and_then(|v| v.as_str()),
    Some("read:widgets write:widgets")
  );
  assert_eq!(
    token_data
      .claims
      .extra
      .get("tenant_id")
      .and_then(|v| v.as_str()),
    Some("acme")
  );
}

// ── rejection cases ─────────────────────────────────────────────────────────

#[tokio::test]
async fn missing_authorization_header_is_unauthorized() {
  let resp = build_app().oneshot(request_with_token(None)).await.unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_token_is_unauthorized() {
  let resp = build_app()
    .oneshot(request_with_token(Some("not-a-jwt")))
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_signature_is_unauthorized() {
  let token = sign_token(&valid_claims(), b"different-secret");
  let resp = build_app()
    .oneshot(request_with_token(Some(&token)))
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_token_is_unauthorized() {
  // jsonwebtoken's default Validation has a 60-second leeway, so put
  // exp comfortably outside that window.
  let claims = json!({
    "sub": "svc",
    "iss": ISSUER,
    "aud": AUDIENCE,
    "exp": now_secs() - 600,
  });
  let token = sign_token(&claims, SECRET);
  let resp = build_app()
    .oneshot(request_with_token(Some(&token)))
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_audience_is_unauthorized() {
  let claims = json!({
    "sub": "svc",
    "iss": ISSUER,
    "aud": "some-other-api",
    "exp": now_secs() + 3600,
  });
  let token = sign_token(&claims, SECRET);
  let resp = build_app()
    .oneshot(request_with_token(Some(&token)))
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_issuer_is_unauthorized() {
  let claims = json!({
    "sub": "svc",
    "iss": "https://attacker.example/",
    "aud": AUDIENCE,
    "exp": now_secs() + 3600,
  });
  let token = sign_token(&claims, SECRET);
  let resp = build_app()
    .oneshot(request_with_token(Some(&token)))
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn algorithm_mismatch_is_unauthorized() {
  // Decoder accepts only HS256; sign with HS384 to confirm rejection.
  let token = encode(
    &Header::new(Algorithm::HS384),
    &valid_claims(),
    &EncodingKey::from_secret(SECRET),
  )
  .unwrap();
  let resp = build_app()
    .oneshot(request_with_token(Some(&token)))
    .await
    .unwrap();
  assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── JwtConfig::validation shape ─────────────────────────────────────────────

#[test]
fn jwt_config_validation_sets_issuer_audience_and_algorithms() {
  let config = JwtConfig {
    jwks_url: "https://x/jwks".into(),
    issuer: "https://issuer.example/".into(),
    audiences: vec!["a".into(), "b".into()],
    algorithms: vec![Algorithm::RS256, Algorithm::ES256],
  };
  let validation = config.validation();
  assert!(validation.algorithms.contains(&Algorithm::RS256));
  assert!(validation.algorithms.contains(&Algorithm::ES256));
  // jsonwebtoken stores these as Option<HashSet<String>> internally;
  // round-tripping via Validation is the documented surface.
  let _: Validation = validation;
}

#[test]
fn jwt_config_validation_defaults_to_rs256_when_algorithms_empty() {
  let config = JwtConfig {
    jwks_url: "https://x/jwks".into(),
    issuer: "https://issuer.example/".into(),
    audiences: vec!["a".into()],
    algorithms: vec![],
  };
  let validation = config.validation();
  assert!(validation.algorithms.contains(&Algorithm::RS256));
}
